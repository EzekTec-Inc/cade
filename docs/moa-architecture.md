# Mixture of Agents (MoA) Architecture

CADE is transitioning to a Mixture of Agents (MoA) architecture to improve modularity, extensibility, and the dynamic delegation of tasks. This document outlines the core components of this new architecture.

## Core Components

### 1. The `Agent` Trait

The foundation of the MoA architecture is the `agent::Agent` trait, located in the `cade-agent` crate. All specialized components, including former "Tools" and "Skills," will implement this trait. It defines a standardized interface for how the central `Core` communicates with each agent.

The trait requires the following methods:

- `fn name(&self) -> &str;`: Returns the unique name of the agent.
- `fn capabilities(&self) -> Vec<String>;`: Returns a vector of strings describing the agent's capabilities. These are used by the `Router` to select the appropriate agent for a task.
- `async fn execute(&self, request: &AgentRequest) -> AgentResult;`: The main execution entry point for the agent. It takes a request and returns a result.

### 2. The `Router`

The `Router` is a component within the `Core` responsible for analyzing incoming prompts and delegating them to the most suitable agents. It maintains a registry of all available agents.

The current routing strategy is to select all agents that have at least one capability matching a keyword in the user's prompt. This allows for multiple agents to be selected for a single prompt, enabling parallel execution.

### 3. The `Aggregator`

The `Aggregator`'s role is to process the results from agents and prepare a final response for the user. With the introduction of parallel agent execution, the `Aggregator` now accepts a list of `AgentResult` objects.

It concatenates the content from each successful result, separated by a markdown horizontal rule (`---`). This allows the LLM to receive a consolidated view of the outcomes from all executed tools in a single turn.

## Agent Implementations

Existing tools are being progressively refactored into `Agent` implementations. For example, the filesystem tools (`read_file`, `write_file`, etc.) have been wrapped in corresponding "ToolAgent" structs (e.g., `ReadToolAgent`) that implement the `Agent` trait. These agents are then registered with the `Router` at runtime.
