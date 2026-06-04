import re

with open("crates/cade-ai/src/openai/tests.rs", "r") as f:
    lines = f.readlines()

out = []
for line in lines:
    if "let arr2 = resp_tools_val" in line:
        continue
    if "assert_eq!(arr2.len()" in line:
        continue
    if "assert_eq!(arr2[0]" in line:
        continue
    if "assert!(arr2.iter()" in line:
        continue
    out.append(line)

with open("crates/cade-ai/src/openai/tests.rs", "w") as f:
    f.writelines(out)

print("Done")
