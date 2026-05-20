import glob, re
for f in glob.glob('/home/engr-uba/.cargo/registry/src/*/tui-textarea-2-*/src/wrap.r' + 's'):
    with open(f) as file:
        text = file.read()
        m = re.search(r'pub enum WrapMode \{(.*?)\}', text, re.DOTALL)
        if m:
            print(m.group(0))
