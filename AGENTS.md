# AGENTS.md

## Desktop app (`crates/bridge75-app`)

### Read the gpui-kit guides first

Before writing or reviewing code in `bridge75-app`, read both guides and follow them:

- Coding: `.agents/skills/gpui-kit/references/coding-guides.md`
- UI/UX: `.agents/skills/gpui-kit-design-guides/references/design-guides.md`

When you ask a subagent to review app code, tell it to check the code against both guides.

### No self-evident text

Do not add descriptions, notes, hints, legends or tooltips that only repeat what the screen
already shows: the page title, a control's label, its current state, or what an obvious
action does.

Keep text only when the user cannot learn it from the screen, such as:

- firmware behavior that is not visible (for example, SOCD pairs follow the keymap)
- instructions the user must follow (for example, the calibration countdown)
- errors and how to recover from them
- names for screen readers

This applies to `crates/bridge75-app/locales/app.yml` and to any new page.
