"""Resolve the tools a contract check's command runs, against this image,
and say whether its verdict comes from running workspace code.

The kernel runs this in a fresh check VM with an empty workspace when a
contract is recorded, so whatever is found here is in the image. Every
program a check runs must come from the image, and nothing may change where
programs and modules are looked up. A check that also
executes a workspace file (a script, tests, a demo, code it imports) is
self-checked: its verdict rests on code the agent wrote. One that only reads
the work with the image's tools is appliance-verified. When in doubt, it is
self-checked.

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
# Image programs that run code from the files or folder they are given.
RUNNERS = {
    "node", "nodejs", "npm", "npx", "perl", "ruby", "php", "java", "make", "cmake", "ninja", "tox", "nox",
    "pytest", "py.test", "git", "xargs", "parallel", "watch", "xvfb-run", "soffice", "libreoffice",
}
# Modules that only read data, never run code from a file.
DATA_MODULES = {"json.tool", "base64", "zipfile", "tarfile", "gzip", "py_compile", "tokenize", "ast", "filecmp", "sqlite3"}
# Names in python3 -c code that can run other code.
LAUNCHERS = {"subprocess", "os.system", "os.popen", "os.exec", "os.spawn", "runpy", "importlib", "exec", "eval", "compile", "__import__", "sys.path", "pty", "multiprocessing", "pickle", "shelve", "marshal", "ctypes", "code"}
# Programs that run a program file when given -f (or --file).
SCRIPTED = {"awk", "gawk", "mawk", "sed", "jq"}


def substitutions(cmd):
    """Pull out $(...) and `...`, outside single quotes; return the command
    with each replaced by a plain word, and the inner commands."""
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
            out.append("SUBSTITUTED")
            i = j
            continue
        if c == "`":
            j = cmd.find("`", i + 1)
            j = len(cmd) if j < 0 else j
            inner.append(cmd[i + 1 : j])
            out.append("SUBSTITUTED")
            i = j + 1
            continue
        out.append(c)
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
    if prog in REFUSED:
        found.problems.append(f"`{prog}` {REFUSED[prog]}")
        return
    if prog == "export":
        for a in args:
            search_var(assignment(a) or a, found)
        return
    if prog in BUILTINS or prog == "SUBSTITUTED":
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
        for j, a in enumerate(args):
            if a == "-c" and j + 1 < len(args):
                if depth > 3:
                    found.problems.append("the command nests shells too deeply to resolve")
                else:
                    check(args[j + 1], depth + 1, found)
                return
            if not a.startswith("-"):
                found.runs.append(f"`{base} {a}` runs a workspace script")
                return
    elif base in RUNNERS:
        found.runs.append(f"`{base}` runs code from the workspace")
    elif base == "find" and set(args) & {"-exec", "-execdir", "-ok", "-okdir"}:
        found.runs.append("`find -exec` runs other programs")
    elif base in SCRIPTED and any(a == "-f" or a.startswith("--file") or (a.startswith("-") and not a.startswith("--") and "f" in a[1:]) for a in args):
        found.runs.append(f"`{base} -f` runs a program file from the workspace")


def check_python(args, found):
    j = 0
    while j < len(args):
        a = args[j]
        if a.startswith("-") and not a.startswith("--") and len(a) > 1 and a[1] not in "mcWX" and set(a[1:]) & {"E", "S"}:
            found.problems.append(f"`python3 {a}` turns off the check environment")
            return
        if a == "-c" or a.startswith("-c"):
            code = args[j + 1] if a == "-c" and j + 1 < len(args) else a[2:]
            check_code(code, found)
            return
        if a == "-m" or (a.startswith("-m") and len(a) > 2):
            mod = args[j + 1] if a == "-m" and j + 1 < len(args) else a[2:]
            if not mod:
                found.problems.append("`python3 -m` names no module")
            elif not image_module(mod):
                found.runs.append(f"`python3 -m {mod}` runs a module from the workspace")
            elif mod not in DATA_MODULES:
                found.runs.append(f"`python3 -m {mod}` runs code from the workspace")
            return
        if a in ("-W", "-X"):
            j += 2
            continue
        if a.startswith("-"):
            j += 1
            continue
        found.runs.append(f"`python3 {a}` runs a workspace script")
        return


def check_code(code, found):
    """python3 -c code is appliance-verified only when it imports nothing
    from outside the image and cannot launch other code."""
    try:
        tree = ast.parse(code)
    except SyntaxError:
        found.runs.append("its python3 -c code cannot be parsed, so what it runs is unknown")
        return
    for node in ast.walk(tree):
        names = []
        if isinstance(node, ast.Import):
            names = [n.name for n in node.names]
        elif isinstance(node, ast.ImportFrom):
            names = [node.module or ""] if node.level == 0 else [""]
        for n in names:
            if not n or not image_module(n.split(".")[0]):
                found.runs.append(f"its python3 -c code imports {n or 'a relative module'}, which is not in the image")
            elif any(n == l or n.startswith(l + ".") for l in LAUNCHERS):
                found.runs.append(f"its python3 -c code imports {n}, which can run other code")
        if isinstance(node, ast.Name) and node.id in LAUNCHERS:
            found.runs.append(f"its python3 -c code uses {node.id}, which can run other code")
        if isinstance(node, ast.Attribute):
            dotted = []
            cur = node
            while isinstance(cur, ast.Attribute):
                dotted.append(cur.attr)
                cur = cur.value
            if isinstance(cur, ast.Name):
                full = ".".join([cur.id] + dotted[::-1])
                if any(full == l or full.startswith(l) for l in LAUNCHERS if "." in l):
                    found.runs.append(f"its python3 -c code uses {full}, which can run other code")


if __name__ == "__main__":
    commands = json.loads(base64.b64decode(sys.argv[1]))
    out = []
    for c in commands:
        f = check(c)
        out.append({"problems": f.problems, "self_checked": list(dict.fromkeys(f.runs))})
    print(json.dumps(out))
