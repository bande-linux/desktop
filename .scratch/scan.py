import os, re, sys
root = sys.argv[1]
lit = re.compile(r'"((?:[^"\\]|\\.)*)"')
logmac = re.compile(r'\b(tracing::\w+!|warn!|error!|info!|debug!|trace!|panic!|unreachable!|assert\w*!|eprintln!|println!|bail!|anyhow!|\.context\(|\.with_context|write!|writeln!)')
for dp, dn, fn in os.walk(os.path.join(root, 'crates')):
    rel = os.path.relpath(dp, root)
    if '/testing/' in '/' + rel + '/' or '/tests' in rel or '/benches' in rel or 'target' in rel:
        continue
    for f in sorted(fn):
        if not f.endswith('.rs') or f == 'tests.rs':
            continue
        p = os.path.join(dp, f)
        lines = open(p).read().split('\n')
        stmt_log = False
        for i, line in enumerate(lines):
            s = line.strip()
            if s.startswith('#[cfg(test)]'):
                break
            if logmac.search(line):
                stmt_log = True
            if stmt_log:
                if s.endswith(';') or s.endswith('),') or s.endswith('{'):
                    stmt_log = False
                continue
            if s.startswith('//'):
                continue
            for m in lit.finditer(line):
                t = m.group(1)
                if len(t) < 2 or not re.search(r'[A-Za-z]', t):
                    continue
                if re.fullmatch(r'[a-z0-9_\-\.:/@%{}]+', t):
                    continue
                if t.startswith(('org.', 'com.', '/', 'x-', 'application/', 'image/', 'text/', 'inode/')):
                    continue
                if re.fullmatch(r'[A-Z_0-9]+', t):
                    continue
                if '\\' in t and ' ' not in t:
                    continue
                if ' ' in t or re.match(r'^[A-Z][a-z]', t) or '…' in t:
                    print(f'{os.path.relpath(p, root)}:{i+1}: "{t}"')
