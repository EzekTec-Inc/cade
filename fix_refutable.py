import os
import glob
import re

def fix_file(filepath):
    with open(filepath, "r") as f:
        content = f.read()

    # Fix overlays.rs: matches!(self, Self::Connected(session) if session.active_question == Some(_))
    content = content.replace("session.active_question == Some(_)", "session.active_question.is_some()")

    # Mutability issues:
    # If the function takes &self (not &mut self), and we have &mut **session
    # we need to change it to &**session.
    # We can detect this roughly by replacing `&mut **session` with `&**session`
    # inside functions that have `(&self` but not `(&mut self`.
    # To be safe, let's just do targeted replacements based on the compiler errors.
    
    if filepath.endswith("context_breakdown.rs"):
        content = content.replace("} = &mut **session;", "} = &**session;")
        
    if filepath.endswith("conversations.rs"):
        # line 17 is in conversations_snapshot(&self)
        content = re.sub(
            r'let crate::session::ConnectedSession \{\s*conversations,\s*\.\.\s*\} = &mut \*\*session;',
            r'let crate::session::ConnectedSession { conversations, .. } = &**session;',
            content
        )
        
    if filepath.endswith("live_output.rs"):
        # line 43 is in live_output_snapshot(&self)
        content = re.sub(
            r'let crate::session::ConnectedSession \{\s*live_outputs,\s*\.\.\s*\} = &mut \*\*session;',
            r'let crate::session::ConnectedSession { live_outputs, .. } = &**session;',
            content
        )

    # Refutable bindings
    if filepath.endswith("checkpoints.rs"):
        # checkpoints_notice
        content = re.sub(
            r'if let Self::Connected\(session\) = self \{\s*'
            r'let crate::session::ConnectedSession \{\s*'
            r'checkpoints_notice: Some\(n\),\s*'
            r'\.\.\s*'
            r'\} = &\*\*session;\s*'
            r'Some\(n\.as_str\(\)\)\s*'
            r'\} else \{\s*'
            r'None\s*'
            r'\}',
            r'''if let Self::Connected(session) = self {
            session.checkpoints_notice.as_deref()
        } else {
            None
        }''',
            content
        )

    if filepath.endswith("memory_overlay.rs"):
        content = re.sub(
            r'let crate::session::ConnectedSession \{\s*'
            r'memory_open: true,\s*'
            r'memory_blocks,\s*'
            r'memory_selection,\s*'
            r'memory_edit_buffer,\s*'
            r'\.\.\s*'
            r'\} = &\*\*session;\s*'
            r'match memory_blocks.get\(\*memory_selection\) \{\s*'
            r'Some\(b\) => b.value != \*memory_edit_buffer,\s*'
            r'None => false,\s*'
            r'\}',
            r'''if session.memory_open {
                match session.memory_blocks.get(session.memory_selection) {
                    Some(b) => b.value != session.memory_edit_buffer,
                    None => false,
                }
            } else {
                false
            }''',
            content
        )
        
        content = re.sub(
            r'let crate::session::ConnectedSession \{\s*'
            r'memory_save_notice: Some\(n\),\s*'
            r'\.\.\s*'
            r'\} = &\*\*session;\s*'
            r'Some\(n\.as_str\(\)\)',
            r'session.memory_save_notice.as_deref()',
            content
        )

    if filepath.endswith("messages.rs"):
        content = re.sub(
            r'if let Self::Connected\(session\) = self \{\s*'
            r'let crate::session::ConnectedSession \{\s*'
            r'selected_agent: Some\(_\),\s*'
            r'\.\.\s*'
            r'\} = &mut \*\*session;\s*'
            r'true\s*'
            r'\} else \{\s*'
            r'false\s*'
            r'\}',
            r'''if let Self::Connected(session) = self {
            session.selected_agent.is_some()
        } else {
            false
        }''',
            content
        )

    if filepath.endswith("overlays.rs"):
        content = re.sub(
            r'if let Self::Connected\(session\) = self \{\s*'
            r'let crate::session::ConnectedSession \{\s*'
            r'active_question: Some\(q\),\s*'
            r'\.\.\s*'
            r'\} = &mut \*\*session;\s*'
            r'Some\(q\)\s*'
            r'\} else \{\s*'
            r'None\s*'
            r'\}',
            r'''if let Self::Connected(session) = self {
            session.active_question.as_ref()
        } else {
            None
        }''',
            content
        )
        
        content = re.sub(
            r'let crate::session::ConnectedSession \{\s*'
            r'active_question: Some\(q\),\s*'
            r'\.\.\s*'
            r'\} = &mut \*\*session;\s*'
            r'q.checked.len\(\)',
            r'session.active_question.as_ref().map(|q| q.checked.len()).unwrap_or(0)',
            content
        )
        
        content = re.sub(
            r'let crate::session::ConnectedSession \{\s*'
            r'active_question: Some\(q\),\s*'
            r'\.\.\s*'
            r'\} = &mut \*\*session;\s*'
            r'let question_checked = &mut q.checked;\s*'
            r'if let Some\(v\) = question_checked\.get_mut\(idx\) \{\s*'
            r'\*v = !\*v;\s*'
            r'\}',
            r'''if let Some(q) = session.active_question.as_mut() {
                if let Some(v) = q.checked.get_mut(idx) {
                    *v = !*v;
                }
            }''',
            content
        )
        
        content = re.sub(
            r'let crate::session::ConnectedSession \{\s*'
            r'active_question: Some\(q\),\s*'
            r'\.\.\s*'
            r'\} = &mut \*\*session;\s*'
            r'q\.multi_select',
            r'session.active_question.as_ref().map(|q| q.multi_select).unwrap_or(false)',
            content
        )


    with open(filepath, "w") as f:
        f.write(content)


for filepath in glob.glob("crates/cade-gui/src/session/*.rs"):
    fix_file(filepath)

print("Fixed refutable bindings and mutability.")
