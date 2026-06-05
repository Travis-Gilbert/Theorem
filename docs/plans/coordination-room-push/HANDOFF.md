# Handoff: The Coordination Room as Primary Surface, plus Native Push

## The reframe

Stop trying to embed Claude Code and Codex inside the app. Make the app a member of the same room the agents are already in. Co-membership, not embedding. The app is one client of the room, the agents are other clients, the room is the shared meeting surface. The thing that was hard, embedding agents in the app, dissolves, because there is nothing to embed.

The substrate already exists: `CoordinationRoom`, `Message`, `Intent`, `Presence`, `Member`, with `COORDINATION_*` edges, and the native impl is confirmed. The read and write verbs already exist too: `coordinate` to post into a room, `mentions` and `read_messages_for_room` to receive. Today this is a side channel while the human drives Claude Code request-response. This handoff promotes it to the primary surface and adds the one primitive the substrate is missing.

## Native push, the missing primitive

The RESP server at `rustyredcore_THG/crates/rustyred-thg-resp-server/src/protocol.rs` is a thin shim that maps four custom commands to graph operations, with no `SUBSCRIBE`, no `PUBLISH`, no keyspace notifications. So RustyRed has no native push. A message write does not emit an event the substrate delivers to anyone; it is a mailbox that does not ring. You do not trigger RustyRed, you trigger the listener. Push is what makes the room feel real-time, and it decomposes into three pieces.

The emit is the keystone. At the coordination-message write path (the `coordinate` write site in the mcp crate where the `Message` node and the mention are created), publish a `RoomMessageEvent { room_id, message_id, author, mentions, delivery }` onto an event bus. Start in-process with a `tokio::sync::broadcast` channel. This needs no external infrastructure. If a consumer ever has to live in a different process, the cross-process bus can be external Redis or NATS, or pub/sub added to RustyRed's own RESP layer; the RustyRed-native route is preferred because it avoids a new dependency. For v1, in-process is enough, and the app connects to the same harness server.

The human real-time side is a WebSocket or SSE endpoint on the harness server that the app subscribes to. Each `RoomMessageEvent` pushes the message down the socket. The app feels live because it holds the socket, not because anything is kept warm server-side.

The agent wake side is a listener task inside the harness server, not a separate always-on process, subscribed to the same bus. On a wake-flagged message that mentions an auto-wake actor, it fires `spawn_session` for that actor. Because the listener is a task in a server that is already running, the marginal always-on cost of the whole push feature is close to nothing: the emit is free, the app socket is one connection per open app, and the listener rides on the server.

Confirm Codex's spawn path so hold-to-wake can target Codex as well as Claude Code. The wake is solid for Claude Code through `spawn_session` on the runner.

## The single send button

One control, two behaviors, both riding the same emit. That sharing is what keeps it clean.

Tap writes the message with `delivery = passive`. The app socket still shows it to anyone watching the room, but the spawn-listener ignores passive messages, so the agent sees it the next time it is live and calls `mentions`. This is the pull path.

Hold writes the message with `delivery = wake`. The spawn-listener fires `spawn` for the mentioned agents, or all room agents if none are named. This is the explicit-trigger path.

A single boolean on the message is the whole difference between leaving a note and queueing the agents. Give a small affordance that the hold registered, a haptic or the button filling, so "I queued them" is legible against "I left a note."

## Boundaries to keep

Keep the coordinate-versus-execute line. Messages coordinate; runs execute. A message like "build X" should launch a run and post the result back into the room, rather than trying to finish inside a chat bubble. That boundary is what keeps the room from becoming a flaky synchronous RPC.

Populate actor identity. Right now claude.ai's writes land with an empty `actor_id`, so a multi-member room would show unattributed messages. For the room to be legible with several members, writes need real author identity. This is the per-agent identity point from the agent-governance work, and it is small but load-bearing for a shared room.

## What v1 is

Promote the room to the primary surface. Build the in-process emit at the coordinate write site. Add the WebSocket or SSE subscription for the app. Add the spawn-listener task that wakes agents on wake-flagged mentions. Ship the single tap-or-hold send button. None of this requires a standing process beyond the harness server already running, and none of it requires external infrastructure. Ambient no-button auto-wake is already covered by this design, since the spawn-listener fires on any wake-flagged message; the button is simply how the human marks intent.
