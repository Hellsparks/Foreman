---
version: alpha
name: Fieldwork AI Implementation Guide
---

## Overview

Instructions for AI coding assistants implementing or extending Fieldwork in this repository or a future app. This file defines the workflow, not another visual specification. Follow the user's requested scope and the host tool's safety and permission rules.

## Read before changing UI

1. Read `DESIGN.md`: the sole authority for Fieldwork's visual and component contracts.
2. Read `tokens.css`: the portable implementation of those contracts.
3. Read `README.md` for adoption instructions and known limitations.
4. Inspect the relevant existing components and state before editing. `App.jsx` is a living reference, not a production component package or a required page layout.

If these disagree, follow DESIGN.md for design intent, report the mismatch, and reconcile affected implementations rather than silently inventing another system. If a required file is missing, disclose it; do not claim Fieldwork compliance from memory.

## Preserve the system

- Reuse semantic `--fw-*` variables and existing components. Do not scatter literal palette, spacing, type, or radius values through feature components.
- Preserve the industrial-minimalist language: structural grids, square panels, restrained highlights, system sans typography, and technical mono labels. Let the product's task determine its layout; do not copy the reference site's composition by default.
- Reuse the opt-in `.fw-*` primitives where appropriate. Read their actual implementation before relying on a class or inventing a new one.
- Preserve existing EDITMODE declarations, bindings, and user-selected values unless the user asks to change them.
- For a genuinely new reusable pattern, first check existing contracts, then define the smallest necessary extension. Keep DESIGN.md, tokens.css, and relevant reference examples aligned. Keep app-specific exceptions local and explicitly documented.
- Do not add fonts, dependencies, copied brand assets, or network integrations just to reproduce the reference's appearance. Fieldwork and its palettes are original interpretations, not official Blacksmith or Marathon kits.

## Themes and modes

- Supported scheme IDs: `fieldwork`, `vector`, `foundry`, `olive`. Supported modes: `light`, `dark`.
- Apply `data-fw-theme` and `data-fw-mode` to the root HTML element in consuming apps. Import tokens.css once using that app's build setup.
- Preserve the consuming app's selected appearance. The reference starts in Fieldwork dark; the attribute-free portable stylesheet falls back to Fieldwork light. Do not silently force one on an existing app.
- Use `--fw-accent-hover` for the base primary-action fill (legacy token name), `--fw-button-hover` for hover, and `--fw-on-accent` for its label. Never assume white labels work in all modes.
- Use semantic status foreground/background pairs. Do not use the decorative accent as a success indicator.
- Controls use `--fw-control-active`, `--fw-control-on`, `--fw-control-off`, and `--fw-control-boundary`. Active highlights follow the scheme; off and disabled states remain neutral.
- Theme changes must not reset form values, filters, selections, or other product state. Do not introduce persistence without a request; handle storage failures if persistence is authorized.

## Implement behavior, not static decoration

- Prefer labelled native buttons and inputs. Group related checkbox/radio options with a fieldset and legend. Use explicit button types inside forms.
- Implement relevant hover, focus-visible, active, selected/checked, disabled, and error states. Convey selection and status through text, shape, or marks as well as color.
- Preserve native keyboard behavior for checkboxes, switches, radios, and ranges. Display slider values with units; derive `--fw-range-fill` from the actual value, minimum, and maximum.
- Share domain records across lists, details, counts, and actions. Preserve relevant state across navigation and provide return/reset or recovery paths.
- Forms must validate accessibly without discarding input. Empty and error states need useful recovery actions.
- If adding dialogs, follow DESIGN.md's focus, accessible naming, dismissal, and focus-restoration contracts; do not treat a visually styled overlay as a complete modal.
- Label sample data and simulated service actions. Do not imply working authentication, payments, delivery, analytics, or backend persistence when none exists.

## Verification before handoff

Check the changed journey, not just the initial render:

- Exercise entry, a meaningful state change, dependent feedback/view, and return or recovery.
- Test keyboard focus and activation, visible labels, and relevant validation and disabled behavior.
- Inspect a desktop and narrow mobile viewport; ensure content and actions remain reachable without horizontal clipping.
- When changing shared styles, inspect all four schemes in both modes. Check contrast for text, focus, boundaries, active marks, and status pairs; do not assume token use guarantees accessibility.
- When changing theme behavior, verify product state survives switching appearance and navigation.
- Use the repository's available checks and runtime preview. Report what was tested, concrete failures, and unavailable checks; never claim an accessibility audit or integration test you did not perform.

## Handoff

Summarize changed files and behavior, verification, and remaining limitations. For future apps, copy this AGENTS.md with DESIGN.md and tokens.css into the project; retain any existing project-specific instructions and merge this guidance rather than overwriting them. Include README.md and App.jsx when useful as setup guidance and examples. A separate SKILL.md is optional, not required to use this system.
