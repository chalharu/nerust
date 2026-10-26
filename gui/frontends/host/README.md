# nerust_frontend_host

Toolkit-free policy owner for native frontends: state, events/effects,
turn order, and wake deadlines. Native pumps call `drive_turn`; widgets,
loops, pickers, and fullscreen execution stay per frontend.

Design record lives in nerust-docs:

- `explanation/frontend-host-boundary.md`
- `reference/frontend-host-contracts.md`
- `how-to-guides/run-host-native-tests.md`
