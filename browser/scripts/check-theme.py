#!/usr/bin/env python3
# check-theme.py <firefox source dir>: fails when theme/bande.css sets a custom property that the
# browser's own stylesheets and scripts never read, as happens when an ESR renames its tokens.
import os
import re
import sys

src = sys.argv[1]
theme = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), 'theme/bande.css')
with open(theme) as sheet:
    ours = {name for name in re.findall(r'(--[\w-]+)\s*:', sheet.read()) if not name.startswith('--bande-')}
read = set()
for top in ('browser', 'toolkit'):
    for root, dirs, files in os.walk(os.path.join(src, top)):
        dirs[:] = [d for d in dirs if d not in ('test', 'tests')]
        for name in files:
            if name.endswith(('.css', '.mjs', '.js')):
                with open(os.path.join(root, name), errors='replace') as source:
                    read.update(re.findall(r'var\(\s*(--[\w-]+)', source.read()))
unread = sorted(ours - read)
if unread:
    sys.exit('check-theme: theme/bande.css sets properties the browser no longer reads: ' + ' '.join(unread))
