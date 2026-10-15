# AGENTS.md — DegenView Rust

Guidance for AI coding agents working in this repository.

## Required reading

Read [Core guidance](docs/agent-guides/core.md) for every task. Then read every guide whose
scenario matches the work:

| Scenario | Read |
| --- | --- |
| Providers, symbols, timeframes, prediction markets, caches, icons, live data, or watchlists | [Market data](docs/agent-guides/market-data.md) |
| Chart rendering, price labels, chart types, canvas, templates, drawings, or portfolio cards | [Charts](docs/agent-guides/charts.md) |
| Pine language/runtime, Script Manager, editor behavior, or example scripts | [Pine and scripts](docs/agent-guides/pine.md) |
| Price/Pine alerts or webhooks | [Alerts and webhooks](docs/agent-guides/alerts-webhooks.md) |
| Tabs, windows, workspaces, saved layouts, session state, or app-wide state | [Tabs and state](docs/agent-guides/tabs-state.md) |
| MCP server, tools, permissions, or clients | [MCP](docs/agent-guides/mcp.md) |
| Releases, signing, packaging, self-update, or update UI | [Releases and updates](docs/agent-guides/releases-updates.md) |

For cross-cutting work, load all matching guides. These files contain implementation constraints,
not general background; do not skip a guide because the code looks self-explanatory.

## Deeper references

- [Architecture](docs/architecture.md) — module map and data flow.
- [Chart types](docs/chart-types.md) — construction rules and Pine semantics.
- [Chart navigation](docs/chart-navigation.md) — viewport, pan/axis gestures, older history, what is unverified.
- [Chart canvas](docs/chart-canvas.md) — settings, geometry, layering, and templates.
- [Drawing tools](docs/drawing-tools.md) — tools, geometry, verified versus inferred behavior, limits.
- [Portfolio cards](docs/portfolio-cards.md) — card design and known gaps.
- [MCP implementation](docs/mcp-implementation.md) and [remaining work](docs/mcp-remaining-work.md).
