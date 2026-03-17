import re

def extract():
    with open("src/main.rs", "r") as f:
        content = f.read()

    agent_resolution = re.search(r"    // Agent resolution — helper closure avoids repeating the create logic.*?upsert_memory\(\n.*?&\w+\.id,\n.*?\"skills\",\n.*?updated_skills_block\.as_deref\(\)\.unwrap_or\(\"\"\),\n.*?None,\n.*?\)\n.*?\.await;\n", content, re.DOTALL)
    
    conversation_resolution = re.search(r"    // ── Conversation resolution ───────────────────────────────────────────────.*?session\.session\.conversation_id\.clone\(\)\n    \};\n", content, re.DOTALL)
    
    if agent_resolution and conversation_resolution:
        print("Found blocks to extract")
    else:
        print("Could not find blocks")
        return

extract()
