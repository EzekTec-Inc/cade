import os
import glob
import re

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
        
    orig_content = content

    # 1. matches!(self, Self::Connected { ... })
    def replace_matches(m):
        full_match = m.group(0)
        fields_str = m.group(1)
        
        # Convert `field: value, ..` to `matches!(&**session, crate::session::ConnectedSession { field: value, .. })`
        return f"matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession {{ {fields_str} }}))"
            
    content = re.sub(r'matches!\(\s*self\s*,\s*Self::Connected\s*\{([^}]+)\}\s*\)', replace_matches, content)

    # 2. if let Self::Connected { ... } = self {
    # We replace it with:
    # if let Self::Connected(session) = self {
    #     if let crate::session::ConnectedSession { ... } = &mut **session {
    # OR &**session depending on `&mut self` vs `&self`

    def replace_if_let(m):
        full_match_start = m.start()
        fields_str = m.group(1)
        
        # Determine if `self` is mut by looking back up to the function signature
        # A simple heuristic: check the nearest `fn ` before this match.
        fn_part = content[:full_match_start]
        fn_idx = fn_part.rfind("fn ")
        
        is_mut = False
        if fn_idx != -1:
            sig = fn_part[fn_idx:]
            # Only consider it &mut self if `&mut self` appears before the opening `{`
            sig_to_brace = sig.split('{')[0]
            if "&mut self" in sig_to_brace:
                is_mut = True
        
        ref_prefix = "&mut **" if is_mut else "&**"
        
        # To avoid extra indentation, we can just replace the pattern match
        # However, Rust allows `if let Self::Connected(session) = self { if let ConnectedSession { ... } = &mut **session {`
        # But this requires adding a closing brace at the end of the block!
        # This is extremely difficult to do with regex alone since we need brace matching.
        
        # ALTERNATIVE approach:
        # Instead of wrapping in another `if let`, we can destructure via `session` variable.
        # But refutable patterns like `memory_open: true` will break `let ConnectedSession { ... } = session`.
        # So brace matching is required to add `}` at the end of the block.
        return f"if let Self::Connected(session) = self {{\n            if let crate::session::ConnectedSession {{ {fields_str} }} = {ref_prefix}session {{"
        
    # We will use brace matching to find the block for `if let` and add `}`
    # Wait, instead of python brace matching, what if we use the fact that `ConnectedSession` is exhaustive except when we have `..`.
    # Actually, brace matching is easy in Python.
    
    # ... Wait, I'll implement a simple brace matcher.
    
    pos = 0
    while True:
        m = re.search(r'if let Self::Connected\s*\{([^}]+)\}\s*=\s*(?:self|&mut self|&self)\s*\{', content[pos:])
        if not m:
            break
            
        match_start = pos + m.start()
        match_end = pos + m.end()
        fields_str = m.group(1)
        
        fn_idx = content[:match_start].rfind("fn ")
        is_mut = False
        if fn_idx != -1:
            sig_to_brace = content[fn_idx:match_start].split('{')[0]
            if "&mut self" in sig_to_brace:
                is_mut = True
                
        ref_prefix = "&mut **" if is_mut else "&**"
        
        # Find closing brace
        open_braces = 1
        curr = match_end
        while curr < len(content) and open_braces > 0:
            if content[curr] == '{':
                open_braces += 1
            elif content[curr] == '}':
                open_braces -= 1
            curr += 1
            
        closing_brace_idx = curr - 1
        
        replacement = f"if let Self::Connected(session) = self {{\n            if let crate::session::ConnectedSession {{ {fields_str} }} = {ref_prefix}session {{"
        
        # Insert closing brace
        content = content[:closing_brace_idx] + "} // close ConnectedSession\n        " + content[closing_brace_idx:]
        
        # Replace opening
        content = content[:match_start] + replacement + content[match_end:]
        
        pos = match_start + len(replacement)
        
    # 3. Replace *self = Self::Connected { ... } with *self = Self::Connected(Box::new(crate::session::ConnectedSession { ... }))
    # This is easy.
    content = re.sub(
        r'\*self\s*=\s*Self::Connected\s*\{',
        r'*self = Self::Connected(Box::new(crate::session::ConnectedSession {',
        content
    )
    # We need to find the matching brace for *self = Self::Connected { and add }))
    pos = 0
    while True:
        m = re.search(r'\*self\s*=\s*Self::Connected\(Box::new\(crate::session::ConnectedSession\s*\{', content[pos:])
        if not m:
            break
        match_start = pos + m.start()
        match_end = pos + m.end()
        
        open_braces = 1
        curr = match_end
        while curr < len(content) and open_braces > 0:
            if content[curr] == '{':
                open_braces += 1
            elif content[curr] == '}':
                open_braces -= 1
            curr += 1
        closing_brace_idx = curr - 1
        
        # Check if we already added it
        if content[closing_brace_idx:closing_brace_idx+3] != "}))":
            content = content[:closing_brace_idx+1] + "))" + content[closing_brace_idx+1:]
            
        pos = match_end

    # Handle `Self::Connected { ... }` in other places, e.g. tests or return values
    # Actually wait, let's just run it.

    if content != orig_content:
        with open(filepath, 'w') as f:
            f.write(content)
        print(f"Updated {filepath}")

for filepath in glob.glob("crates/cade-gui/src/session/*.rs"):
    process_file(filepath)
