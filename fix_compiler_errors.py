import sys, re

output = sys.stdin.read()
errors = re.findall(r"--> (.*?):(\d+):(\d+)", output)

# Map of file -> list of line numbers
fixes = {}
for file, line, _ in errors:
    # only care about missing field 'ui_resource_uri'
    if "missing field `ui_resource_uri`" in output:
        fixes.setdefault(file, []).append(int(line))

for file, lines in fixes.items():
    try:
        with open(file, "r") as f:
            content = f.read().splitlines()
    except FileNotFoundError:
        continue
        
    # Sort lines descending so we can modify without throwing off line numbers
    for line in sorted(set(lines), reverse=True):
        # The line is usually where `ToolResult {` is. We need to find the matching `}`.
        idx = line - 1
        brace_count = 0
        found_open = False
        insert_idx = -1
        for i in range(idx, len(content)):
            if "{" in content[i]:
                brace_count += content[i].count("{")
                found_open = True
            if "}" in content[i]:
                brace_count -= content[i].count("}")
                if found_open and brace_count == 0:
                    insert_idx = i
                    break
        
        if insert_idx != -1:
            content.insert(insert_idx, "ui_resource_uri: None,")
            
    with open(file, "w") as f:
        f.write("\n".join(content) + "\n")
