# Forge Crate Renaming Plan

## Current Structure
```
forge-agents   → agents
forge-cli      → cli (or keep forge-cli as main binary)
forge-context  → context
forge-core     → core
forge-ext      → ext
forge-provider → provider
forge-sandbox  → sandbox
forge-verify   → verify
```

## Dependency Graph
```
cli
├── core
│   ├── provider
│   ├── context
│   └── sandbox
├── provider
├── context
├── sandbox
└── verify
    └── agents
```

## Rename Order (bottom-up to avoid breaking)
1. agents (leaf, no dependencies)
2. ext (leaf, no dependencies)
3. provider (leaf, no dependencies)
4. sandbox (leaf, no dependencies)
5. context (leaf, no dependencies)
6. verify (depends on agents)
7. core (depends on provider, context, sandbox)
8. cli (depends on all)

## Steps per Crate
1. Rename directory
2. Update Cargo.toml (name, dependencies)
3. Update all imports in code
4. Update parent dependencies
5. Test build

