# Termfold terminfo

`termfold-256color.terminfo` is the public inner-terminal contract. The compiled
entry is checked in at `compiled/t/termfold-256color` and embedded in the binary.
Regenerate it on Linux or WSL with:

```sh
tic -x -o terminfo/compiled terminfo/termfold-256color.terminfo
```

## Capability checks

| Capability group | Focused check |
| --- | --- |
| `am`, `xenl`, cursor movement, tabs, editing, erase, insert/delete, and scrolling | `terminal::tests::parses_text_width_cursor_editing_and_scrolling` |
| SGR attributes, 256/RGB colours, alternate screen, cursor visibility, and application-cursor mode | `terminal::tests::tracks_sgr_buffers_and_input_modes` |
| Cursor addressing, device reports, and fixed-size resize semantics | `terminal::tests::emits_standard_terminal_reports_and_resizes_without_reflow` |
| `TERM`, `COLORTERM`, and private `TERMINFO` environment | `pty::tests::shell_inherits_context_and_pty_resizes` |
| Embedded-byte equality, private mode, atomic creation, and invalid-entry rejection | `runtime::tests::terminfo_is_materialized_atomically_and_rejects_invalid_entries` |
