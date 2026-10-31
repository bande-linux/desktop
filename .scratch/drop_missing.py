import re, subprocess, sys

files = subprocess.run(['grep', '-rl', 'missing_ids', 'crates', '--include=*.rs'], capture_output=True, text=True).stdout.split()
for path in files:
    if path.startswith('crates/libs/l10n'):
        continue
    lines = open(path).read().split('\n')
    idx = next(i for i, l in enumerate(lines) if 'missing_ids' in l)
    start = idx
    while not lines[start].strip().startswith('#[test]'):
        start -= 1
    depth = 0
    end = start + 1
    opened = False
    while True:
        depth += lines[end].count('{') - lines[end].count('}')
        if '{' in lines[end]:
            opened = True
        if opened and depth == 0:
            break
        end += 1
    del lines[start:end + 1]
    if start < len(lines) and lines[start].strip() == '' and lines[start - 1].strip() == '':
        del lines[start]
    mod = start - 1
    while mod >= 0 and not re.match(r'\s*mod tests \{', lines[mod]):
        mod -= 1
    depth = 0
    close = mod
    while True:
        depth += lines[close].count('{') - lines[close].count('}')
        if depth == 0:
            break
        close += 1
    body = [l for l in lines[mod + 1:close] if l.strip() and not l.strip().startswith('use ')]
    if not body:
        first = mod - 1 if lines[mod - 1].strip() == '#[cfg(test)]' else mod
        del lines[first:close + 1]
        while first > 0 and lines[first - 1].strip() == '' and (first >= len(lines) or lines[first].strip() == ''):
            del lines[first - 1]
            first -= 1
    else:
        while start - 1 >= 0 and lines[start - 1].strip() == '' and lines[start].strip() in ('}', ''):
            del lines[start - 1]
            start -= 1
    text = '\n'.join(lines)
    if not text.endswith('\n'):
        text += '\n'
    open(path, 'w').write(text)
    print(path, 'module removed' if not body else 'test removed')
