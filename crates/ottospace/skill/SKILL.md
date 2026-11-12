---
name: ottospace
description: Drive the ottospace app from the command line: open and arrange terminal panes and tabs, type into terminals, read their output, and bring app windows into panes. Use when the user asks to do something in ottospace, or when a task needs a terminal pane or a tab the user can watch.
---

# ottospace

Run this and follow the guide it prints. It is the guide for the installed version:

```sh
ottospace skill
```

If that fails with "command not found", ottospace is not installed or not on `PATH`: tell the user.
If a later `ottospace ctl` call says "no ottospace at ...", the app is not running: ask the user
to start it. Do not start it yourself.
