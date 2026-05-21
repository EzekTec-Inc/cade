sed -i 's/SessionState::Connected {/SessionState::Connected(session) => { let crate::session::ConnectedSession {/g' crates/cade-gui/src/session/tests.rs
sed -i 's/if let SessionState::Connected(session) => {/if let SessionState::Connected(session) =/g' crates/cade-gui/src/session/tests.rs
sed -i 's/if let SessionState::Connected {/if let SessionState::Connected(session) = \&mut s { let crate::session::ConnectedSession {/g' crates/cade-gui/src/session/tests.rs
