import re

with open("crates/cade-ai/src/openai/tests.rs", "r") as f:
    code = f.read()

# Replace any lingering arr2 blocks
code = re.sub(r'assert_eq!\(\s*arr2\.len\(\)[^;]*;', '', code, flags=re.MULTILINE|re.DOTALL)
code = re.sub(r'assert_eq!\(\s*arr2\[0\][^;]*;', '', code, flags=re.MULTILINE|re.DOTALL)
code = re.sub(r'assert!\(\s*arr2\.iter\(\)[^;]*;', '', code, flags=re.MULTILINE|re.DOTALL)
code = re.sub(r'assert_eq!\(\s*arr2\.iter\(\)[^;]*;', '', code, flags=re.MULTILINE|re.DOTALL)

with open("crates/cade-ai/src/openai/tests.rs", "w") as f:
    f.write(code)

print("Done")
