# Servo engine PRs: the interactive accessibility tree

Date: 2026-06-08. Plan home: docs/plans/servo-browser-use-agent/. Companion to HANDOFF.md (bd0b7e1d) and build-step-1-parity.md (job-007), which consumes this work through the PageState reader.

Posture, Travis's call: these land as commits on the fork plus harness notes. The upstream proposal to the Servo team is HELD until the build is further along. Structure each PR as a clean, independently mergeable branch so it can be peeled into an upstream PR later without rework. Target repo: Travis-Gilbert/servo (create the fork from servo/servo if it does not exist yet). Everything gates behind the existing `--pref accessibility_enabled`.

## Grounding (components/layout/accessibility_tree.rs, SHA 3c6a6f23)

The tree is built inside layout via AccessKit, which gives two assets for free: geometry is at hand (the box tree exists where the a11y tree is built) and `finalize()` already emits an `accesskit::TreeUpdate` containing only changed nodes, a native change signal. `id_for_opaque` assigns one persistent NodeId per DOM node, so stable element identity exists today.

The gaps these PRs close: `HTML_ELEMENT_ROLE_MAPPINGS` covers fifteen structural tags (article, aside, body, footer, h1 through h6, header, hr, main, nav, p); everything interactive falls to `Role::GenericContainer`. No bounds on any node. No value, checked, or disabled state on form fields. `NAME_FROM_CONTENTS_ROLES` contains only Heading. `ActionRequest` appears in a doc comment with no perform_action handler anywhere.

Upstream context for later: the active team is alice, delan, lukewarlow (Igalia); reference PRs #42338, #44255, #44208 when the proposal unfreezes.

## PR 1: interactive role mappings

Extend `HTML_ELEMENT_ROLE_MAPPINGS` and `role_from_dom_node` to cover: button (Button), a with href (Link), input by type (TextInput, CheckBox, RadioButton, Button for submit/reset, and the obvious siblings: SearchInput, PasswordInput where AccessKit roles exist), select (ComboBox single, ListBox multiple), textarea (MultilineTextInput), option (ListBoxOption), label, form, img (Image), table elements where cheap. Read the ARIA `role` attribute on the element and let an explicit valid role override the implicit mapping.

Files: components/layout/accessibility_tree.rs. Acceptance: a fixture page with a form produces a TreeUpdate where the button, link, text input, checkbox, and select carry their interactive roles instead of GenericContainer, and an element with `role="button"` on a div reports Button.

## PR 2: interactive node properties

Populate AccessKit node state from DOM state during `update_node`: value for form fields, checked and selected, disabled, required, placeholder, focusable, expanded where applicable. AccessKit `Node` has setters for all of these; follow the existing set_role/set_value pattern (compare, set, mark updated) so the incremental diff stays correct.

Files: accessibility_tree.rs plus whatever layout_dom accessors are needed to read element state. Acceptance: typing into an input produces a TreeUpdate whose node carries the new value; toggling a checkbox flips checked; a disabled button reports disabled.

## PR 3: accessible name algorithm

Expand `NAME_FROM_CONTENTS_ROLES` beyond Heading to Button, Link, and the other name-from-content roles per the ARIA accname spec, and implement the resolution order: aria-label, aria-labelledby, label[for] association, alt on images, title fallback, then contents. The existing `label_from_descendants` walk is the base; the attribute branches are new.

Files: accessibility_tree.rs. Acceptance: a button with aria-label reports that label; an input wrapped by label[for] reports the label text; an icon link with no text but a title reports the title.

## PR 4: bounds from layout

Attach the layout fragment's border-box rect to each AccessKit node via set_bounds, in page coordinates, documented as such (AccessKit models bounds as a rect plus optional transform; start with absolute page space). The tree is built in layout, so the fragment is reachable at update time; offscreen and zero-size nodes still carry their rects so the reader can compute visibility.

Files: accessibility_tree.rs plus the fragment lookup it needs. Acceptance: every rendered interactive node in a fixture page carries a nonempty bbox that matches its painted position; a node scrolled out of the viewport carries coordinates outside the viewport rect.

## PR 5: action support

The heaviest and the one that turns perception into actuation. Implement supported_actions per role (Click on activatable elements, Focus on focusables, SetValue on editable fields, ScrollIntoView generally) and a perform_action path that routes an incoming `accesskit::ActionRequest` to real DOM activation: Click runs the element's activation behavior, Focus moves document focus, SetValue updates the field and fires input events. This crosses from layout into script (layout cannot mutate the DOM), so the routing goes through the existing layout-to-script channel; first verify whether any perform_action plumbing exists in the fork or in webview_delegate, since the layout file references ActionRequest but implements no handler. This is the same capability screen readers and voice control need, which is exactly why it is upstreamable later, and it is the engine-native act() primitive the executor in build step one drives.

Files: accessibility_tree.rs, the script-side activation handler, the embedder plumbing that delivers ActionRequests. Acceptance: an ActionRequest Click on a button fires its click handler in-page; SetValue on an input updates the value and the subsequent TreeUpdate reflects it; Focus moves focus observably.

## Sequencing and tests

PRs 1 through 4 are layout-local and largely independent; land in order 1, 2, 4, 3 if convenient (bounds early helps the reader). PR 5 is last and coordinated as its own branch. Extend components/servo/tests/accessibility.rs alongside each PR to upstream quality, and write one harness note per landed branch (branch name, what it adds, fixture used) so the later upstream peel is mechanical.

## Fences

- Thin maintained patch set tracking servo/servo main by rebase, not a divergent fork.
- No agent-specific types in the engine: the engine speaks AccessKit; PageState, occlusion logic, and everything agent-shaped stays in rustyred-web per build step one.
- Upstream proposal stays held until Travis says otherwise.
- No time estimates, no em dashes in docs.
