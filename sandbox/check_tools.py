"""Resolve the tools a contract check's command runs, against this image,
and say whether its verdict comes from running workspace code.

The kernel runs this in a fresh check VM with an empty workspace when a
contract is recorded, so whatever is found here is in the image. Every
program a check runs must come from the image, and nothing may change where
programs and modules are looked up. A check that also
executes a workspace file (a script, tests, a demo, code it imports) is
self-checked: its verdict rests on code the agent wrote. One that only reads
the work with the image's tools is appliance-verified. When in doubt, it is
self-checked: a check is appliance-verified only when every program it runs
is one known to read what it is given (READERS), and every program text it
hands an interpreter (python3 -c, sh -c, awk, sed, jq) is fixed when the
contract is recorded and uses only what is known not to run other code.

usage: python3 -I check_tools.py BASE64_JSON_LIST_OF_COMMANDS
Prints a JSON list holding, for each command,
{"problems": [...], "self_checked": [reasons it runs workspace code]}.
"""
import ast
import base64
import importlib.util
import json
import os
import re
import shlex
import shutil
import sys

PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
# Variables that change where programs or modules come from, or switch off
# the check environment.
SEARCH = {
    "PATH", "HOME", "IFS", "CDPATH", "ENV", "BASH_ENV", "SHELLOPTS", "MAGNUS_CHECK",
    "LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "NODE_PATH", "NODE_OPTIONS",
    "PERL5LIB", "PERL5OPT", "RUBYLIB", "RUBYOPT",
}
BUILTINS = {"cd", "test", "[", "echo", "printf", "true", "false", "exit", ":", "set", "unset", "shift", "wait", "read", "pwd", "umask", "export", "return"}
REFUSED = {
    ".": "sources a file into the checking shell",
    "source": "sources a file into the checking shell",
    "eval": "builds a command the check cannot be resolved from",
    "alias": "changes what a name runs",
    "hash": "changes what a name runs",
    "case": "is not supported in a check; use test or a python3 -c snippet",
}
KEYWORDS = {"if", "then", "else", "elif", "fi", "do", "done", "while", "until", "!", "{", "}"}
# Wrappers that run the program named after their own options.
WRAPPERS = {"env", "timeout", "time", "nice", "nohup", "stdbuf", "ionice", "command", "exec", "xvfb-run"}
SEPARATORS = {";", "&&", "||", "|", "&", "(", ")", "\n", ";;", "|&"}
REDIRECTS = {"<", ">", ">>", "<<", "<<<", ">&", "<&", ">|", "&>", "&>>", "<>"}
PYTHON = re.compile(r"^python(\d+(\.\d+)?)?$")
SHELLS = {"sh", "bash", "dash"}
# Image programs that only read what they are given and run nothing else.
# Any other program a check runs makes it self-checked. Interpreters and the
# programs with a way to run commands are looked at below.
READERS = {
    "cat", "head", "tail", "wc", "grep", "egrep", "fgrep", "diff", "cmp", "comm", "ls", "stat", "file",
    "sha1sum", "sha256sum", "sha512sum", "md5sum", "cksum", "uniq", "cut", "tr", "paste", "join", "fold",
    "nl", "od", "rev", "tac", "strings", "iconv", "basename", "dirname", "realpath", "readlink", "du", "df",
    "date", "seq", "expr", "tee", "true", "false", "echo", "printf", "test", "[", "env", "pwd", "id", "uname",
    "pdftotext", "pdfinfo", "pdffonts", "pdftoppm", "pdfseparate", "qpdf", "unzip", "zipinfo", "zcat",
    "gzip", "gunzip", "xz", "unxz", "xzcat", "bzip2", "bunzip2", "bzcat", "tesseract", "sleep",
}
# Modules that only read data, never run code from a file (python3 -m).
DATA_MODULES = {"json.tool", "base64", "zipfile", "tarfile", "gzip", "py_compile", "tokenize", "ast", "filecmp", "sqlite3"}
# Modules python3 -c code may import and stay appliance-verified: none of
# them runs code from a file, imports a module named in data, or evaluates
# a string. os and sys only through the attributes below.
SAFE_MODULES = {
    "json", "re", "math", "statistics", "string", "textwrap", "difflib", "csv", "hashlib", "base64", "binascii",
    "zipfile", "gzip", "bz2", "lzma", "zlib", "tarfile", "datetime", "time", "collections", "itertools",
    "functools", "decimal", "fractions", "unicodedata", "struct", "glob", "fnmatch", "filecmp", "html",
    "html.parser", "xml", "xml.etree", "xml.etree.ElementTree", "xml.dom", "xml.dom.minidom", "configparser",
    "tomllib", "io", "codecs", "pathlib", "os", "os.path", "sys", "email", "mimetypes", "pprint", "shlex",
    "random", "uuid", "ast", "tokenize", "enum", "openpyxl", "docx", "pptx", "pypdf", "pdfplumber", "fitz",
    "bs4", "lxml", "lxml.etree", "lxml.html", "tabulate",
}
OS_SAFE = {
    "path", "listdir", "scandir", "walk", "stat", "lstat", "getcwd", "sep", "linesep", "environ", "getenv",
    "access", "R_OK", "W_OK", "X_OK", "F_OK", "fspath", "cpu_count",
}
SYS_SAFE = {"argv", "exit", "stdin", "stdout", "stderr", "version", "version_info", "platform", "maxsize", "byteorder", "float_info", "implementation"}
RESTRICTED = {"os": OS_SAFE, "sys": SYS_SAFE}
# Builtins that run, import or reach code by name.
UNSAFE_NAMES = {
    "exec", "eval", "compile", "__import__", "getattr", "setattr", "delattr", "globals", "locals", "vars",
    "breakpoint", "help", "__builtins__", "__loader__", "__spec__",
}
# Programs whose program text decides what they do, and the options that
# load that text, or code, from a file.
AWKS = {"awk", "gawk", "mawk"}
# Marks a shell expansion outside single quotes ($NAME, ${...}, $(...),
# `...`): what it stands for is known only when the check runs.
EXPANDED = "\x01"
SUBSTITUTED = EXPANDED + "SUB"


def shown(s):
    return s.replace(SUBSTITUTED, "$(...)").replace(EXPANDED, "$")


def substitutions(cmd):
    """Pull out $(...) and `...`, outside single quotes; return the command
    with each replaced by a marked word and every other $ outside single
    quotes marked, and the inner commands."""
    out, inner, i, quote = [], [], 0, None
    while i < len(cmd):
        c = cmd[i]
        if quote == "'":
            if c == "'":
                quote = None
            out.append(c)
            i += 1
            continue
        if c == "'" and quote is None:
            quote = "'"
        elif c == '"':
            quote = None if quote == '"' else '"'
        if cmd.startswith("$(", i) and not cmd.startswith("$((", i):
            depth, j = 1, i + 2
            while j < len(cmd) and depth:
                depth += {"(": 1, ")": -1}.get(cmd[j], 0)
                j += 1
            inner.append(cmd[i + 2 : j - 1])
            out.append(SUBSTITUTED)
            i = j
            continue
        if c == "`":
            j = cmd.find("`", i + 1)
            j = len(cmd) if j < 0 else j
            inner.append(cmd[i + 1 : j])
            out.append(SUBSTITUTED)
            i = j + 1
            continue
        out.append(EXPANDED if c == "$" else c)
        i += 1
    return "".join(out), inner


def simple_commands(cmd):
    lex = shlex.shlex(cmd, posix=True, punctuation_chars="();<>|&\n")
    lex.whitespace = " \t\r"
    lex.whitespace_split = True
    lex.commenters = "#"
    words, current, skip_next = [], [], False
    for tok in lex:
        if skip_next:
            skip_next = False
            continue
        if tok in SEPARATORS:
            if current:
                words.append(current)
            current = []
        elif tok in REDIRECTS or (set(tok) <= set("<>&|") and tok):
            if current and current[-1].isdigit():
                current.pop()
            skip_next = True
        else:
            current.append(tok)
    if current:
        words.append(current)
    return words


def assignment(word):
    m = re.match(r"^([A-Za-z_][A-Za-z0-9_]*)=", word)
    return m.group(1) if m else None


def image_program(name):
    """Where `name` runs from, if the image has it."""
    if "/" in name:
        p = os.path.realpath(name if name.startswith("/") else os.path.join("/workspace", name))
        if p.startswith(("/workspace", "/tmp", "/home", "/root", "/dev/shm")):
            return None
        return p if os.path.isfile(p) and os.access(p, os.X_OK) else None
    return shutil.which(name, path=PATH)


def image_module(name):
    try:
        spec = importlib.util.find_spec(name)
    except (ImportError, ValueError):
        return False
    origin = spec and (spec.origin or (spec.submodule_search_locations and list(spec.submodule_search_locations)[0]))
    return bool(spec) and (origin in ("built-in", "frozen") or os.path.realpath(str(origin)).startswith("/usr/"))


class Found:
    def __init__(self):
        self.problems = []
        self.runs = []


def check(cmd, depth=0, found=None):
    found = found or Found()
    flat, inner = substitutions(cmd)
    for sub in inner:
        check(sub, depth + 1, found)
    try:
        commands = simple_commands(flat)
    except ValueError as e:
        found.problems.append(f"the command cannot be parsed ({e})")
        return found
    for words in commands:
        check_simple(words, depth, found)
    return found


def search_var(name, found):
    if name in SEARCH or name.startswith("PYTHON"):
        found.problems.append(f"it sets {name}, which changes where the check's tools come from")


def built(text, what, found):
    """Program text the shell expands is known only when the check runs."""
    if EXPANDED in text:
        found.runs.append(f"its {what} is built by the shell when the check runs, so what it runs is not known in advance")
        return True
    return False


def check_simple(words, depth, found):
    i = 0
    while i < len(words) and words[i] in KEYWORDS:
        i += 1
    if i < len(words) and words[i] == "for":
        return
    # Leading assignments, then wrappers with their options and assignments.
    while i < len(words):
        w = words[i]
        name = assignment(w)
        if name:
            search_var(name, found)
            i += 1
            continue
        if w in WRAPPERS:
            if not image_program(w) and w not in ("command", "exec", "time"):
                found.problems.append(f"`{w}` is not in the sandbox image")
            i += 1
            first = True
            while i < len(words) and (words[i].startswith("-") or assignment(words[i]) or (w in ("timeout", "nice") and first and re.match(r"^[0-9.]+[smhd]?$", words[i]))):
                name = assignment(words[i])
                if name:
                    search_var(name, found)
                if not words[i].startswith("-"):
                    first = False
                i += 1
            continue
        break
    if i >= len(words):
        return
    prog, args = words[i], words[i + 1 :]
    if EXPANDED in prog:
        found.problems.append(f"`{shown(prog)}` names the program it runs only when the check runs; name an image program")
        return
    if prog in REFUSED:
        found.problems.append(f"`{prog}` {REFUSED[prog]}")
        return
    if prog == "export":
        for a in args:
            search_var(assignment(a) or a, found)
        return
    if prog in BUILTINS:
        return
    if not image_program(prog):
        if "/" in prog:
            found.problems.append(f"`{prog}` would run a workspace file directly, and a check VM does not execute workspace files; run it with its interpreter, such as `sh {prog}` or `python3 {prog}`")
        else:
            found.problems.append(f"`{prog}` is not in the sandbox image")
        return
    base = os.path.basename(prog)
    if PYTHON.match(base):
        check_python(args, found)
    elif base in SHELLS:
        check_shell(base, args, depth, found)
    elif base in AWKS:
        check_awk(base, args, found)
    elif base == "sed":
        check_sed(args, found)
    elif base == "jq":
        check_jq(args, found)
    elif base == "find":
        if set(args) & {"-exec", "-execdir", "-ok", "-okdir"}:
            found.runs.append("`find -exec` runs other programs")
    elif base == "sort":
        if any(a.startswith("--compress-program") for a in args):
            found.runs.append("`sort --compress-program` runs another program")
    elif base not in READERS:
        found.runs.append(f"`{base}` is not one of the image's programs known only to read what they are given, so it may run workspace code")


def check_shell(base, args, depth, found):
    for j, a in enumerate(args):
        if a.startswith("-") and not a.startswith("--") and "c" in a[1:]:
            if j + 1 >= len(args):
                found.problems.append(f"`{base} {a}` has no command")
            elif built(args[j + 1], "shell code", found):
                pass
            elif depth > 3:
                found.problems.append("the command nests shells too deeply to resolve")
            else:
                check(args[j + 1], depth + 1, found)
            return
        if a == "-s" or a == "-":
            break
        if not a.startswith("-"):
            found.runs.append(f"`{base} {shown(a)}` runs a workspace script")
            return
    found.runs.append(f"`{base}` runs the commands it reads from its input")


def check_python(args, found):
    j = 0
    while j < len(args):
        a = args[j]
        if a.startswith("-") and not a.startswith("--") and len(a) > 1 and a[1] not in "mcWX" and set(a[1:]) & {"E", "S"}:
            found.problems.append(f"`python3 {a}` turns off the check environment")
            return
        if a == "-c" or a.startswith("-c"):
            code = args[j + 1] if a == "-c" and j + 1 < len(args) else a[2:]
            if not built(code, "python3 -c code", found):
                check_code(code, found)
            return
        if a == "-m" or (a.startswith("-m") and len(a) > 2):
            mod = args[j + 1] if a == "-m" and j + 1 < len(args) else a[2:]
            if not mod:
                found.problems.append("`python3 -m` names no module")
            elif EXPANDED in mod:
                found.problems.append(f"`python3 -m {shown(mod)}` names its module only when the check runs")
            elif not image_module(mod):
                found.runs.append(f"`python3 -m {mod}` runs a module from the workspace")
            elif mod not in DATA_MODULES:
                found.runs.append(f"`python3 -m {mod}` runs code from the workspace")
            return
        if a in ("-W", "-X"):
            # A warning filter imports the module that names its category.
            if a == "-W" and j + 1 < len(args) and "." in args[j + 1]:
                found.runs.append(f"`python3 -W {shown(args[j + 1])}` imports the module its category names")
            j += 2
            continue
        if a == "-":
            break
        if a.startswith("-"):
            j += 1
            continue
        found.runs.append(f"`python3 {shown(a)}` runs a workspace script")
        return
    found.runs.append("`python3` runs the code it reads from its input")


def check_code(code, found):
    """python3 -c code is appliance-verified only when it imports only
    modules known not to run other code, and reaches os and sys only through
    attributes known to be safe; anything else is self-checked."""
    try:
        tree = ast.parse(code)
    except SyntaxError:
        found.runs.append("its python3 -c code cannot be parsed, so what it runs is unknown")
        return
    parents = {}
    for node in ast.walk(tree):
        for child in ast.iter_child_nodes(node):
            parents[child] = node
    # Names bound to os or sys, which may only be used through safe attributes.
    restricted = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for n in node.names:
                top = n.name.split(".")[0]
                if top in RESTRICTED and (n.asname is None or n.name == top):
                    restricted[n.asname or top] = top
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for n in node.names:
                safe_module(n.name, found)
        elif isinstance(node, ast.ImportFrom):
            mod = node.module or ""
            if node.level or not mod:
                found.runs.append("its python3 -c code imports a relative module, which is not in the image")
                continue
            safe_module(mod, found)
            if mod in RESTRICTED:
                for n in node.names:
                    if n.name not in RESTRICTED[mod]:
                        found.runs.append(f"its python3 -c code imports {mod}.{n.name}, which can run other code")
        elif isinstance(node, ast.Name):
            if node.id in UNSAFE_NAMES:
                found.runs.append(f"its python3 -c code uses {node.id}, which can run other code")
            elif node.id in restricted:
                mod = restricted[node.id]
                p = parents.get(node)
                if not (isinstance(p, ast.Attribute) and p.value is node and p.attr in RESTRICTED[mod]):
                    attr = p.attr if isinstance(p, ast.Attribute) and p.value is node else None
                    found.runs.append(f"its python3 -c code uses {mod}.{attr}, which can run other code" if attr else f"its python3 -c code uses {mod} other than through known attributes")
        elif isinstance(node, ast.Attribute) and node.attr.startswith("__"):
            found.runs.append(f"its python3 -c code reaches {node.attr}, which can reach code by name")
        elif isinstance(node, ast.Constant) and isinstance(node.value, str) and "__" in node.value:
            found.runs.append("its python3 -c code holds a string naming a special attribute, which can reach code by name")


def safe_module(name, found):
    """A module is safe when it, or the package it is in, is in SAFE_MODULES;
    of os and sys, only os.path."""
    parts = name.split(".")
    if not image_module(parts[0]):
        found.runs.append(f"its python3 -c code imports {name}, which is not in the image")
    elif parts[0] in RESTRICTED and name not in SAFE_MODULES:
        found.runs.append(f"its python3 -c code imports {name}, which can run other code")
    elif not any(".".join(parts[:k]) in SAFE_MODULES for k in range(1, len(parts) + 1)):
        found.runs.append(f"its python3 -c code imports {name}, which is not known to only read data")


def check_awk(base, args, found):
    """awk program text can run commands (system, pipes) and load code
    (@include, @load, -f, -i, -l)."""
    j = 0
    while j < len(args):
        a = args[j]
        if a in ("-F", "-v"):
            j += 2
            continue
        if a.startswith("--file") or a.startswith("--include") or a.startswith("--load") or a.startswith("--exec") or (a.startswith("-") and not a.startswith("--") and set(a[1:2]) & set("fEil")):
            found.runs.append(f"`{base} {a}` runs a program file from the workspace")
            return
        if a.startswith("-"):
            j += 1
            continue
        if not built(a, f"{base} program", found) and re.search(r"system|\||@|getline", a):
            found.runs.append(f"its {base} program can run commands or load code")
        return


# sed scripts known to only print or delete lines: addresses, then p, d, q,
# n or =, or a substitution with / that neither runs (e) nor writes (w).
SED_SAFE = re.compile(r"^\s*(?:(?:\d+|\$)(?:\s*,\s*(?:\d+|\$))?|/[^/\n]*/)?\s*!?\s*(?:[pdqn=]|s/(?:[^/\\]|\\.)*/(?:[^/\\]|\\.)*/[gpiI0-9]*)\s*$")


def check_sed(args, found):
    """GNU sed runs commands with its e command and s///e; --sandbox turns
    that off."""
    scripts, j, sandboxed = [], 0, "--sandbox" in args
    while j < len(args):
        a = args[j]
        if a in ("-f",) or a.startswith("--file") or (a.startswith("-") and not a.startswith("--") and "f" in a[1:]):
            found.runs.append("`sed -f` runs a program file from the workspace")
            return
        if a == "-e" or a == "--expression":
            if j + 1 < len(args):
                scripts.append(args[j + 1])
            j += 2
            continue
        if a.startswith("--expression="):
            scripts.append(a.split("=", 1)[1])
        elif not a.startswith("-") and not scripts:
            scripts.append(a)
            break
        j += 1
    for s in scripts:
        if built(s, "sed script", found):
            return
        if not sandboxed and not all(SED_SAFE.match(part) for part in s.split(";") if part.strip()):
            found.runs.append("its sed script is not one known only to print or delete lines, and a sed script can run commands (its e command); add --sandbox")
            return


def check_jq(args, found):
    """jq runs no commands, but can load its program or modules from files."""
    j = 0
    while j < len(args):
        a = args[j]
        if a in ("-f", "--from-file", "-L") or a.startswith("--library-path") or a.startswith("-L"):
            found.runs.append(f"`jq {a}` loads its program from the workspace")
            return
        if a in ("--arg", "--argjson", "--slurpfile", "--rawfile"):
            j += 3
            continue
        if a.startswith("-"):
            j += 1
            continue
        if not built(a, "jq program", found) and re.search(r"\b(import|include)\b", a):
            found.runs.append("its jq program loads a module from the workspace")
        return


if __name__ == "__main__":
    commands = json.loads(base64.b64decode(sys.argv[1]))
    out = []
    for c in commands:
        f = check(c)
        out.append({"problems": [shown(p) for p in f.problems], "self_checked": list(dict.fromkeys(shown(r) for r in f.runs))})
    print(json.dumps(out))
