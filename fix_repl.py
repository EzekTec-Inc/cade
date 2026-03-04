import re

with open('src/cli/repl.rs', 'r') as f:
    text = f.read()

# Fix "?esult" and "?es" and "?r"
text = re.sub(r'app\.ask_question\(&([^)]+)\)\?esult', r'app.ask_question(&\1)?', text)
text = re.sub(r'app\.ask_question\(&([^)]+)\)\?es', r'app.ask_question(&\1)?', text)
text = re.sub(r'app\.ask_question\(&([^)]+)\)\?r', r'app.ask_question(&\1)?', text)

with open('src/cli/repl.rs', 'w') as f:
    f.write(text)

print("Fixed repl.rs")
