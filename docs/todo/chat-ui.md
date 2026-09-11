# Chat UI

Main conversation surface.

## Done

- [x] Virtualized message list (MessageScroller) with follow-tail + jump button
- [x] User messages: right-aligned accent bubble
- [x] Assistant messages: left-aligned bubble + "Rixl" bot header
- [x] Tool-call cards: icon + name + status icon (running/done/failed)
- [x] "Working…" strip while running

## Todo

- [ ] Markdown rendering in assistant text
- [ ] Tool-call card: collapsible detail (command, args, output)
- [ ] Diff cards: file path, +/- counts, inline diff view
- [ ] Streaming text: token-by-token append with caret
- [ ] Elapsed-time in Working strip; Stop button there too
- [ ] Message footer actions: copy, retry, thumbs up/down
- [ ] Empty state: centered prompt suggestions
- [ ] Error banner on failed run with retry
- [ ] Code blocks with syntax highlight + copy button
