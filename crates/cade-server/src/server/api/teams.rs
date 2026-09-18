//! Teams and Swarm Topology API endpoints.

use axum::{Json, http::StatusCode, response::IntoResponse};
use cade_api_types::{SwarmTopologyResponse, TeamMemberSummary, TeamSummary};
use std::env;

/// GET /v1/teams or GET /v1/swarm/topology
///
/// Discovers and returns all teams, subagents, and supervisory hierarchies.
pub async fn get_swarm_topology_handler() -> impl IntoResponse {
    let cwd = env::current_dir().unwrap_or_default();
    let discovered_teams = cade_agent::team::discovery::discover_all_teams(&cwd);
    let standalone_subs = cade_agent::subagents::discover_all_subagents(&cwd);

    let mut teams: Vec<TeamSummary> = Vec::new();
    let mut total_nodes = 0;

    for t in discovered_teams {
        let members: Vec<TeamMemberSummary> = t
            .members
            .into_iter()
            .map(|m| {
                total_nodes += 1;
                TeamMemberSummary {
                    id: m.id,
                    name: m.name,
                    role: m.role,
                    description: m.description,
                    model: m.model,
                    tools: format!("{}", m.tools),
                    status: "Ready".to_string(),
                }
            })
            .collect();

        teams.push(TeamSummary {
            id: t.id,
            name: t.name,
            description: t.description,
            mode: format!("{}", t.mode),
            max_iterations: t.max_iterations,
            leader_model: t.leader_model,
            members,
            scope: format!("{}", t.scope),
        });
    }

    let standalone_subagents: Vec<TeamMemberSummary> = standalone_subs
        .into_iter()
        .map(|s| {
            total_nodes += 1;
            TeamMemberSummary {
                id: s.name.clone(),
                name: s.name,
                role: Some("Autonomous Subagent".to_string()),
                description: s.description,
                model: s.model,
                tools: "Configured".to_string(),
                status: "Idle".to_string(),
            }
        })
        .collect();

    let resp = SwarmTopologyResponse {
        teams,
        standalone_subagents,
        total_nodes,
    };

    (StatusCode::OK, Json(resp))
}
