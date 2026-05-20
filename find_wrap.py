import glob, re
for f in glob.glob('/home/engr-uba/.cargo/registry/src/*/tui-textarea-2-*/src/textarea.r' + 's'):
    with open(f) as file:
        text = file.read()
        for m in re.finditer(r'pub fn set_wrap.*?\)', text):
            print(m.group(0))
