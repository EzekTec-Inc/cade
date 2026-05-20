import glob, re
for f in glob.glob('/home/engr-uba/.cargo/registry/src/*/tui-textarea-2-*/src/wrap.r' + 's'):
    with open(f) as file:
        text = file.read()
        for m in re.finditer(r'pub enum WrapMode \{.*?', text, re.DOTALL):
            print(m.group(0)[:200])
