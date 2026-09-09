# Learning Record 0009: Deployment Views & Infrastructure Mapping

- **Date**: 2026-09-08
- **Topic**: C4 Level 4 Deployment Views (Cloud, Infrastructure & Container Instances)
- **Status**: Verified

## Key Insights
1. **Software vs Infrastructure Separation**:
   - The static C4 model (`model { ... }`) describes software systems, containers, and components independently of where they run.
   - The deployment architecture maps those containers onto physical or cloud infrastructure across different environments (`Production`, `Staging`, `Development`).
2. **Core DSL Constructs**:
   - `deploymentEnvironment "<name>" { ... }`: Groups infrastructure for a specific target environment.
   - `deploymentNode "<name>" [desc] [tech] [instances]`: Hierarchically represents cloud providers, regions, VPCs, subnets, Kubernetes clusters, namespaces, or bare-metal machines.
   - `infrastructureNode "<name>" [desc] [tech]`: Represents pure infrastructure appliances that don't execute custom software containers (e.g. AWS Route 53, ALB, API Gateway, Firewalls).
   - `containerInstance <container-ref>` / `softwareSystemInstance <system-ref>`: Deploys an actual instance of a previously modeled software container or system into that deployment node.
3. **Automatic Relationship Propagation**:
   - Relationships established between containers in the static model are automatically inherited and visualised between their corresponding `containerInstance` nodes in the deployment view!
4. **Hierarchical Scoping**:
   - Under `!identifiers hierarchical`, references across deployment nodes traverse parent-child paths (e.g. `region.vpc.eks.pods.apiPod`).
