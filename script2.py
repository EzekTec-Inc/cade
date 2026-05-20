
import sys
import glob

for file in glob.glob('crates/cade-gui/src/**/*.r'+'s', recursive=True):
    with open(file) as f:
        lines = f.readlines()
    for i, l in enumerate(lines):
        if 'password(true)' in l:
            print(f'{file}:{i}: {l.strip()}')

