import os
import re

replacements = {
    "claude-sonnet-4-5-20250929": "claude-3-5-sonnet-20241022",
    "claude-haiku-4-5": "claude-3-5-haiku-20241022",
    "claude-opus-4-5": "claude-3-opus-20240229",
    "claude-sonnet-4-6": "claude-3-7-sonnet-20250219",
    "claude-sonnet-4-20250514": "claude-3-5-sonnet-20241022",
    "gpt-4.1": "gpt-4-turbo",
    "GPT-4.1": "GPT-4-Turbo",
    "o4-mini": "o1-mini",
    "o4 Mini": "o1-mini",
    "Claude Opus 4.5": "Claude 3 Opus",
    "Claude Sonnet 4.6": "Claude 3.7 Sonnet",
    "Claude Sonnet 4.5": "Claude 3.5 Sonnet",
    "Claude Haiku 4.5": "Claude 3.5 Haiku",
}

for root, dirs, files in os.walk("."):
    for dir_name in [".git", "target", "node_modules"]:
        if dir_name in dirs:
            dirs.remove(dir_name)
    
    for file in files:
        if not file.endswith((".rs", ".md", ".json", ".toml", ".sh")):
            continue
        
        filepath = os.path.join(root, file)
        try:
            with open(filepath, "r", encoding="utf-8") as f:
                content = f.read()
        except:
            continue
            
        new_content = content
        for k, v in replacements.items():
            new_content = new_content.replace(k, v)
            
        if new_content != content:
            with open(filepath, "w", encoding="utf-8") as f:
                f.write(new_content)
            print(f"Updated {filepath}")

