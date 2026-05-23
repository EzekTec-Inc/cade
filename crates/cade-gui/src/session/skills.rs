//! Skills-list state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Called when GET /v1/providers returns.
    pub fn on_providers_loaded(&mut self, _providers: Vec<serde_json::Value>) {
        // The providers overlay currently uses a static list from the palette.
        // When we add a dynamic providers list field, store it here.
        // For now this is a no-op — the fetch exists so the overlay can be
        // populated later without adding another spawn call.
    }

    /// Called when GET /v1/skills + GET /v1/agents/:id/skills return.
    pub fn on_skills_loaded(&mut self, all: Vec<crate::api::SkillEntry>, loaded_ids: Vec<String>) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                all_skills_list,
                loaded_skill_ids,
                skills_loading,
                ..
            } = &mut **session;
            *all_skills_list = all;
            *loaded_skill_ids = loaded_ids;
            *skills_loading = false;
        }
    }

    /// Called after POST /v1/agents/:id/skills/load succeeds.
    pub fn on_skill_loaded(&mut self, id: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                loaded_skill_ids, ..
            } = &mut **session;
            if !loaded_skill_ids.contains(&id.to_string()) {
                loaded_skill_ids.push(id.to_string());
            }
        }
    }

    /// Called after POST /v1/agents/:id/skills/unload succeeds.
    pub fn on_skill_unloaded(&mut self, id: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                loaded_skill_ids, ..
            } = &mut **session;
            loaded_skill_ids.retain(|x| x != id);
        }
    }
}
