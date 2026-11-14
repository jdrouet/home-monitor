# ADR-0007: Server-rendered dashboard

Status: accepted (revisit if interactivity needs grow)

## Decision
`dashboard` renders HTML on the server (axum, a template engine, htmx) and
draws charts with uPlot. No WASM toolchain, no JS build step.
