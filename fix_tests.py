import re

with open('crates/cade-gui/src/session/tests.rs', 'r') as f:
    content = f.read()

# Instead of blindly doing regex, let's fix the specific assert macros manually
content = content.replace('SessionState::Connected { health, .. } => assert_eq!(health.status, "ok"),', 
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { health, .. } = &**session; assert_eq!(health.status, "ok"); },')

content = content.replace('SessionState::Connected { agents, .. } => assert!(agents.is_empty()),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { agents, .. } = &**session; assert!(agents.is_empty()); },')

content = content.replace('SessionState::Connected { artifacts_busy, .. } => assert!(*artifacts_busy),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { artifacts_busy, .. } = &**session; assert!(*artifacts_busy); },')

content = content.replace('SessionState::Connected { artifacts_busy, .. } => assert!(!*artifacts_busy),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { artifacts_busy, .. } = &**session; assert!(!*artifacts_busy); },')

content = content.replace('SessionState::Connected { tools_loading, .. } => assert!(*tools_loading),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { tools_loading, .. } = &**session; assert!(*tools_loading); },')


def fix_match(m):
    return 'SessionState::Connected(session) => {\n            let crate::session::ConnectedSession { ' + m.group(1) + ' } = &**session;\n'
content = re.sub(r'SessionState::Connected\s*\{\s*([^}]+)\s*\}\s*=>\s*\{', fix_match, content)

def fix_if_mut(m):
    return 'if let SessionState::Connected(session) = &mut s {\n        let crate::session::ConnectedSession { ' + m.group(1) + ' } = &mut **session;'
content = re.sub(r'if let SessionState::Connected\s*\{\s*([^}]+)\s*\}\s*=\s*&mut\s*s\s*\{', fix_if_mut, content)

def fix_if_ref(m):
    return 'if let SessionState::Connected(session) = &s {\n        let crate::session::ConnectedSession { ' + m.group(1) + ' } = &**session;'
content = re.sub(r'if let SessionState::Connected\s*\{\s*([^}]+)\s*\}\s*=\s*&s\s*\{', fix_if_ref, content)

# Hand fix the remaining assert_eq!(match) assignments 
content = content.replace('SessionState::Connected { memory_error, .. } => memory_error.clone(),',
                          'SessionState::Connected(session) => session.memory_error.clone(),')

with open('crates/cade-gui/src/session/tests.rs', 'w') as f:
    f.write(content)


content = content.replace('SessionState::Connected { checkpoints_loading, .. } => assert!(!*checkpoints_loading),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { checkpoints_loading, .. } = &**session; assert!(!*checkpoints_loading); },')

content = content.replace('SessionState::Connected { question_cursor, .. } => assert_eq!(*question_cursor, 0),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { question_cursor, .. } = &**session; assert_eq!(*question_cursor, 0); },')

content = content.replace('SessionState::Connected { question_cursor, .. } => assert_eq!(*question_cursor, 2),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { question_cursor, .. } = &**session; assert_eq!(*question_cursor, 2); },')

content = content.replace('SessionState::Connected { context_loading, .. } => assert!(*context_loading),',
                          'SessionState::Connected(session) => { let crate::session::ConnectedSession { context_loading, .. } = &**session; assert!(*context_loading); },')

with open('crates/cade-gui/src/session/tests.rs', 'w') as f:
    f.write(content)

