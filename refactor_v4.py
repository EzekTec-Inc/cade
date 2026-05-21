import os
import glob
import re

def process_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
        
    orig_content = content

    # Pre-fix known refutable patterns manually
    if "checkpoints.rs" in filepath:
        content = re.sub(
            r'if let Self::Connected \{\s*checkpoints_notice: Some\(n\),\s*\.\.\s*\} = self \{\s*Some\(n\.as_str\(\)\)\s*\} else \{\s*None\s*\}',
            r'if let Self::Connected(session) = self { session.checkpoints_notice.as_deref() } else { None }',
            content
        )
    if "memory_overlay.rs" in filepath:
        content = re.sub(
            r'if let Self::Connected \{\s*memory_open: true,\s*memory_blocks,\s*memory_selection,\s*memory_edit_buffer,\s*\.\.\s*\} = self \{\s*match memory_blocks\.get\(\*memory_selection\) \{\s*Some\(b\) => b\.value != \*memory_edit_buffer,\s*None => false,\s*\}\s*\} else \{\s*false\s*\}',
            r'''if let Self::Connected(session) = self {
            if session.memory_open {
                return match session.memory_blocks.get(session.memory_selection) {
                    Some(b) => b.value != session.memory_edit_buffer,
                    None => false,
                };
            }
        }
        false''',
            content
        )
        content = re.sub(
            r'if let Self::Connected \{\s*memory_open: true,\s*memory_blocks,\s*memory_selection,\s*memory_edit_buffer,\s*\.\.\s*\} = self \{\s*match memory_blocks\.get\(\*memory_selection\) \{\s*Some\(b\) => b\.value == \*memory_edit_buffer,\s*None => false,\s*\}\s*\} else \{\s*false\s*\}',
            r'''if let Self::Connected(session) = self {
            if session.memory_open {
                return match session.memory_blocks.get(session.memory_selection) {
                    Some(b) => b.value == session.memory_edit_buffer,
                    None => false,
                };
            }
        }
        false''',
            content
        )
        content = re.sub(
            r'if let Self::Connected \{\s*memory_save_notice: Some\(n\),\s*\.\.\s*\} = self \{\s*Some\(n\.as_str\(\)\)\s*\} else \{\s*None\s*\}',
            r'if let Self::Connected(session) = self { session.memory_save_notice.as_deref() } else { None }',
            content
        )
    if "messages.rs" in filepath:
        content = re.sub(
            r'if let Self::Connected \{\s*selected_agent: Some\(_\),\s*\.\.\s*\} = self \{\s*true\s*\} else \{\s*false\s*\}',
            r'if let Self::Connected(session) = self { session.selected_agent.is_some() } else { false }',
            content
        )
    if "overlays.rs" in filepath:
        content = re.sub(
            r'matches!\(\s*self\s*,\s*Self::Connected\s*\{\s*active_question: Some\(_\),\s*\.\.\s*\}\s*\)',
            r'matches!(self, Self::Connected(session) if session.active_question.is_some())',
            content
        )
        content = re.sub(
            r'if let Self::Connected \{\s*active_question: Some\(q\),\s*\.\.\s*\} = self \{\s*Some\(q\)\s*\} else \{\s*None\s*\}',
            r'if let Self::Connected(session) = self { session.active_question.as_ref() } else { None }',
            content
        )
        content = re.sub(
            r'if let Self::Connected \{\s*active_question: Some\(q\),\s*\.\.\s*\} = self \{\s*q\.checked\.len\(\)\s*\} else \{\s*0\s*\}',
            r'if let Self::Connected(session) = self { session.active_question.as_ref().map(|q| q.checked.len()).unwrap_or(0) } else { 0 }',
            content
        )
        content = re.sub(
            r'if let Self::Connected \{\s*active_question: Some\(q\),\s*\.\.\s*\} = self \{\s*let question_checked = &mut q\.checked;\s*if let Some\(v\) = question_checked\.get_mut\(idx\) \{\s*\*v = !\*v;\s*\}\s*\}',
            r'''if let Self::Connected(session) = self {
            if let Some(q) = session.active_question.as_mut() {
                if let Some(v) = q.checked.get_mut(idx) {
                    *v = !*v;
                }
            }
        }''',
            content
        )
        content = re.sub(
            r'if let Self::Connected \{\s*active_question: Some\(q\),\s*\.\.\s*\} = self \{\s*q\.multi_select\s*\} else \{\s*false\s*\}',
            r'if let Self::Connected(session) = self { session.active_question.as_ref().map(|q| q.multi_select).unwrap_or(false) } else { false }',
            content
        )


    # 1. matches!(self, Self::Connected { ... })
    def replace_matches(m):
        full_match = m.group(0)
        fields_str = m.group(1)
        return f"matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession {{ {fields_str} }}))"
            
    content = re.sub(r'matches!\(\s*self\s*,\s*Self::Connected\s*\{([^}]+)\}\s*\)', replace_matches, content)

    # 2. if let Self::Connected { ... } = self {
    def replace_if_let(m):
        full_match_start = m.start()
        fields_str = m.group(1)
        
        # Determine if `self` is mut
        fn_part = content[:full_match_start]
        fn_idx = fn_part.rfind("fn ")
        
        is_mut = False
        if fn_idx != -1:
            sig_to_brace = fn_part[fn_idx:].split('{')[0]
            if "&mut self" in sig_to_brace:
                is_mut = True
        
        ref_prefix = "&mut **" if is_mut else "&**"
        
        return f"if let Self::Connected(session) = self {{\n            let crate::session::ConnectedSession {{ {fields_str} }} = {ref_prefix}session;"

    content = re.sub(r'if let Self::Connected\s*\{([^}]+)\}\s*=\s*self\s*\{', replace_if_let, content)
    
    # 3. Replace *self = Self::Connected { ... } with *self = Self::Connected(Box::new(crate::session::ConnectedSession { ... }))
    content = re.sub(
        r'\*self\s*=\s*Self::Connected\s*\{',
        r'*self = Self::Connected(Box::new(crate::session::ConnectedSession {',
        content
    )
    # find matching brace and append `))`
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
        
        if content[closing_brace_idx:closing_brace_idx+3] != "}))":
            content = content[:closing_brace_idx+1] + "))" + content[closing_brace_idx+1:]
            
        pos = match_end

    if content != orig_content:
        with open(filepath, 'w') as f:
            f.write(content)

for filepath in glob.glob("crates/cade-gui/src/session/*.rs"):
    process_file(filepath)
