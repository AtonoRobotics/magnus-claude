"""Resolve the tools a contract check's command runs, against this image.

The kernel runs this in a fresh check VM with an empty workspace when a
contract is recorded, so whatever is found here is in the image. A check may
run workspace code as its subject (a file handed to an interpreter or a shell),
never as the tool doing the checking: every program, sourced file and
`python -m` module must come from the image, and nothing may change where they
are looked up.

usage: python3 -I check_tools.py BASE64_JSON_LIST_OF_COMMANDS
Prints a JSON list holding, for each command, a list of problems.
"""
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


def check(cmd, depth=0):
    problems = []
    flat, inner = substitutions(cmd)
    for sub in inner:
        problems += check(sub, depth + 1)
    try:
        commands = simple_commands(flat)
    except ValueError as e:
        return problems + [f"the command cannot be parsed ({e})"]
    for words in commands:
        problems += check_simple(words, depth)
    return problems


def check_simple(words, depth):
    problems = []
    i = 0
    while i < len(words) and words[i] in KEYWORDS:
        i += 1
    if i < len(words) and words[i] == "for":
        return problems
    # Leading assignments, then wrappers with their options and assignments.
    while i < len(words):
        w = words[i]
        name = assignment(w)
        if name:
            if name in SEARCH or name.startswith("PYTHON"):
                problems.append(f"it sets {name}, which changes where the check's tools come from")
            i += 1
            continue
        if w in WRAPPERS:
            if not image_program(w) and w not in ("command", "exec", "time"):
                problems.append(f"`{w}` is not in the sandbox image")
            i += 1
            first = True
            while i < len(words) and (words[i].startswith("-") or assignment(words[i]) or (w in ("timeout", "nice") and first and re.match(r"^[0-9.]+[smhd]?$", words[i]))):
                name = assignment(words[i])
                if name and (name in SEARCH or name.startswith("PYTHON")):
                    problems.append(f"it sets {name}, which changes where the check's tools come from")
                if not words[i].startswith("-"):
                    first = False
                i += 1
            continue
        break
    if i >= len(words):
        return problems
    prog, args = words[i], words[i + 1 :]
    if prog in REFUSED:
        return problems + [f"`{prog}` {REFUSED[prog]}"]
    if prog == "export":
        for a in args:
            name = assignment(a) or a
            if name in SEARCH or name.startswith("PYTHON"):
                problems.append(f"it sets {name}, which changes where the check's tools come from")
        return problems
    if prog in BUILTINS or prog == "SUBSTITUTED":
        return problems
    if not image_program(prog):
        if "/" in prog:
            problems.append(f"`{prog}` is a program from the workspace; a check's tools must come from the sandbox image (run a workspace file through one, e.g. `sh {prog}` or `python3 {prog}`)")
        else:
            problems.append(f"`{prog}` is not in the sandbox image")
        return problems
    base = os.path.basename(prog)
    if PYTHON.match(base):
        problems += check_python(args)
    elif base in SHELLS:
        for j, a in enumerate(args):
            if a == "-c" and j + 1 < len(args):
                if depth > 3:
                    problems.append("the command nests shells too deeply to resolve")
                else:
                    problems += check(args[j + 1], depth + 1)
                break
            if not a.startswith("-"):
                break
    return problems


def check_python(args):
    j = 0
    while j < len(args):
        a = args[j]
        if a.startswith("-") and not a.startswith("--") and len(a) > 1 and a[1] not in "mcWX" and set(a[1:]) & {"E", "S"}:
            return [f"`python3 {a}` turns off the check environment"]
        if a == "-c" or a.startswith("-c"):
            return []
        if a == "-m" or (a.startswith("-m") and len(a) > 2):
            mod = args[j + 1] if a == "-m" and j + 1 < len(args) else a[2:]
            if not mod:
                return ["`python3 -m` names no module"]
            if not image_module(mod):
                return [f"`python3 -m {mod}`: {mod} is not in the sandbox image; a check's tools must come from the image (run workspace code by path, e.g. `python3 path/to/script.py`)"]
            return []
        if a in ("-W", "-X"):
            j += 2
            continue
        if a.startswith("-"):
            j += 1
            continue
        return []  # a script: the subject the check runs
    return []


if __name__ == "__main__":
    commands = json.loads(base64.b64decode(sys.argv[1]))
    print(json.dumps([check(c) for c in commands]))
