import os, re, glob
root = '.'
for ftl in sorted(glob.glob('crates/*/*/locales/en-US/*.ftl')):
    crate = ftl.split('/locales/')[0]
    text = open(ftl).read()
    ids = re.findall(r'^([a-zA-Z][a-zA-Z0-9_-]*)\s*=', text, re.M)
    refs = set(re.findall(r'\{\s*-?([a-zA-Z][a-zA-Z0-9_-]*)\s*[}.]', text))
    src = ''
    for p in glob.glob(crate + '/src/**/*.rs', recursive=True):
        src += open(p).read()
    lits = set(re.findall(r'"((?:[^"\\]|\\.)*)"', src))
    pats = [re.compile('^' + re.sub(r'\\\{[^}]*\\\}', '[a-z0-9-]+', re.escape(l)) + '$') for l in lits if '{' in l and re.fullmatch(r'[a-z0-9{}_:.\- ]+', l)]
    calls = re.findall(r'\btr(?:_args)?\(\s*"([^"]*)"', src)
    missing = [c for c in calls if c not in ids]
    unused = [i for i in ids if i not in lits and i not in refs and not any(p.match(i) for p in pats)]
    print(crate, len(ids), 'missing', missing, 'unused', unused)
