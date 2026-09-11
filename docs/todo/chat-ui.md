# Chat UI

Main conversation surface.

## Done

- [x] Virtualized message list (MessageScroller) with follow-tail + jump button
- [x] User messages: right-aligned accent bubble
- [x] Assistant messages: left-aligned bubble + "Rixl" bot header
- [x] Tool-call cards: icon + name + status icon (running/done/failed)
- [x] "Working…" strip while running
- [x] Markdown rendering in assistant text
- [x] Tool-call card: collapsible detail (command, args, output)
- [x] Diff cards: file path, +/- counts, inline diff view
- [x] Streaming text: token-by-token append with caret
- [x] Elapsed-time in Working strip; Stop button there too
- [x] Message footer actions: copy, retry, thumbs up/down
- [x] Empty state: centered prompt suggestions
- [x] Error banner on failed run with retry
- [x] Message timestamps (HH:MM) in footer
- [x] "Regenerate" on last assistant message
- [x] Header overflow menu: pin, rename, export, copy transcript

## Todo

- [ ] Code blocks with syntax highlight + copy button
- [x] Word-wrap toggle for long lines
- [ ] Message search within chat
