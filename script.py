
import sys
with open('crates/cade-gui/src/app/mod.r'+'s') as f:
    lines = f.readlines()
for i, l in enumerate(lines):
    if 'password(true)' in l:
        for j in range(max(0, i-5), min(len(lines), i+5)):
            print(lines[j], end='')

