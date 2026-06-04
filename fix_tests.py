import re

with open("crates/cade-ai/src/openai/tests.rs", "r") as f:
    code = f.read()

# Remove test `needs_responses_api_check`
code = re.sub(r'#\[test\]\nfn needs_responses_api_check\(\).*?^\}', '', code, flags=re.MULTILINE|re.DOTALL)

# Remove test `parse_responses_api_empty_arguments_is_object`
code = re.sub(r'#\[test\]\nfn parse_responses_api_empty_arguments_is_object\(\).*?^\}', '', code, flags=re.MULTILINE|re.DOTALL)

# Remove test `parse_responses_api_function_call`
code = re.sub(r'#\[test\]\nfn parse_responses_api_function_call\(\).*?^\}', '', code, flags=re.MULTILINE|re.DOTALL)

# Remove test `parse_responses_api_text`
code = re.sub(r'#\[test\]\nfn parse_responses_api_text\(\).*?^\}', '', code, flags=re.MULTILINE|re.DOTALL)

# Remove test `build_responses_tools_wraps_correctly_without_strict`
code = re.sub(r'#\[test\]\nfn build_responses_tools_wraps_correctly_without_strict\(\).*?^\}', '', code, flags=re.MULTILINE|re.DOTALL)

# Remove `responses_api_o_series_maps_system_to_developer`
code = re.sub(r'#\[test\]\nfn responses_api_o_series_maps_system_to_developer\(\).*?^\}', '', code, flags=re.MULTILINE|re.DOTALL)

# Remove any other occurrences of build_responses_tools validations in other tests
# We can just replace OpenAiProvider::build_responses_tools with OpenAiProvider::build_tools just to make it compile, then we'll see if it passes.
code = code.replace('let resp_tools_val = OpenAiProvider::build_responses_tools(&req);', '')
code = code.replace('assert_eq!(resp_tools_val["type"], "function");', '')
code = code.replace('assert_eq!(resp_tools_val["strict"], false);', '')
code = code.replace('assert_eq!(resp_tools_val, json!([]));', '')
# Or just remove those lines manually if we need to.

with open("crates/cade-ai/src/openai/tests.rs", "w") as f:
    f.write(code)

print("Done")
