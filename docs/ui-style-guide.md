# UI style guide

This guide describes the browser UI as it exists today. `web/app.css`, `web/app.js`, and the language files are the source of truth. Read this page before you change a page, an overlay, or a string. The guide is not a redesign: a visual change needs its own issue.

## Principles

- The UI stays quiet and readable. Content comes first, decoration second.
- German is the default language. Every visible string is a key in `web/i18n/de.json` and `web/i18n/en.json`. Nothing is hard-coded, and the two catalogs list the same keys.
- Use the shared helpers (`shell`, `table`, `pill`, `openDialog`, `openEditDialog`) before you write markup of your own.
- New colors and sizes come from the tokens in `web/app.css`. Do not add raw colors or one-off spacing to a page.

## Layout and tokens

The signed-in UI uses the `.shell` grid: a 232 px sidebar and a flexible main column. `.main` carries the page padding and the `#main` landmark. The sidebar holds `.side`, `.nav`, `.nav-group`, and `.nav-label`. A page starts with `.top`, which holds the `h1` and page actions. The sidebar foot (`.side-foot`) sits at the bottom and holds the account link, the build line with the repository link, the language switch, and the sign-out button.

All design tokens are custom properties on `:root` in `web/app.css`.

### Colors

| Token | Use |
| --- | --- |
| `--ink` | Body text and dark surfaces |
| `--paper` | Page background |
| `--panel` | Cards, dialogs, and controls |
| `--line` | Borders and table rules |
| `--muted` | Secondary text and help text |
| `--accent` | Links, focus rings, primary buttons, selected tabs |
| `--accent-soft` | Chips and selected choices |
| `--good`, `--good-bg` | Success banner and successful status |
| `--bad`, `--bad-bg` | Error banner, destructive button, failed status |
| `--warn`, `--warn-bg` | Warning banner and pending status |
| `--info`, `--info-bg` | Running status and the workflow inspector |
| `--side`, `--side-text` | Sidebar background and text |
| `--shadow` | Card, menu, and dialog elevation |

### Typography

- `--sans` is the body font. The base size is 15 px with a 1.45 line height.
- `--serif` is for the page title (`h1`), the wordmark, and large statistics.
- `--mono` is for commands, log output, IDs, and paths. Use it through `code`, `.mono`, or `.tool`.

### Shape and spacing

- Text fields and selects use an 8 px radius. `.btn` uses 9 px, the ticket-like `.btn.pill` and status `.pill` use a full radius, `.banner` uses 10 px, `.card` uses 14 px, and `.overlay-card` uses 16 px.
- Prefer spacing steps of 4, 8, 10, 12, 16, and 24 px. `web/app.css` already uses 2, 6, 14, 18, 20, and 28 px in a few places; do not add a new value when one of the preferred steps fits.
- `.btn` is 7 px by 12 px with 8 px to the next button. `.btn.compact` is 4 px by 8 px for table rows.
- `.field` stacks a label and a control with a 4 px gap. `.form` stacks fields with 10 px. `.form-section` separates groups with a top border and 14 px above the first field.

## Components

### Buttons

Use `.btn` for a normal action, `.btn.primary` for the one main action on a page or in a dialog, `.btn.danger` for a destructive action, `.btn.ghost` for a quiet action, and `.btn.pill` for a rounded action. `.btn.compact` fits a table row. Disabled buttons use the `disabled` attribute.

```html
<button class="btn" type="button">Cancel</button>
<button class="btn primary" type="submit">Save</button>
<button class="btn danger" type="button">Delete</button>
<button class="btn ghost" type="button" aria-label="Close">×</button>
<button class="btn compact" type="button">Edit</button>
<a class="btn pill" href="#/runtime?edit=ID">Edit</a>
<button class="btn primary" type="button" disabled>Save</button>
```

Use a link with `.btn` when it navigates (`<a class="btn" ...>`). Use a `button` with `type="button"` when it runs code, and `type="submit"` only inside a form.

A destructive action asks first. Existing dialogs use `confirm(t("delete_this_...", { name: item.name }))`; an edit dialog can also put the delete button in `.dialog-danger`. Do not delete a named object on a single click.

### Fields

Every control sits in a `<label class="field">`. The label text is the first `span`. Add the `required` class and the `required` attribute when the field is required; the class adds the red asterisk. Help text is a `.small.muted` paragraph or span. Field errors use `.field-error`; the script adds it with `aria-live="polite"` and sets `aria-invalid` on the control.

```html
<label class="field required">
  <span>Name</span>
  <input name="name" required>
</label>
<label class="field">
  <span>Description</span>
  <input name="description">
  <span class="small muted">Shown in the list.</span>
  <span class="field-error"></span>
</label>
```

Use `.field-row` for a few related controls that fit on one line and `.field-grid` for a responsive two-column group. Use `.form-section` for a titled group inside a long form.

```html
<div class="field-grid">
  <label class="field"><span>Project</span><select name="project_id"></select></label>
  <label class="field"><span>Inventory</span><select name="inventory_id"></select></label>
</div>
<div class="form-section">
  <h2>Options</h2>
  <label class="field"><span>Verbosity</span><select class="fit" name="verbosity"></select></label>
</div>
```

Use `.check` for one checkbox, `.check-grid` for a group of checkboxes, and `.check-row` for a checkbox with its label. Use `.chips`, `.chip`, `.chip-remove`, and `.chip-entry` for a list of typed values such as tags. Use `.switch` for an on/off control. Widths are explicit classes: `.fit`, `.tiny`, and `.chips-field`; do not set widths in a `style` attribute.

### Lists and tables

Every table goes through the `table` helper or reproduces its markup: `.table-wrap` outside, `.table` inside, and an empty state. The helper renders the empty state as a muted row when there are no rows.

```html
<div class="table-wrap">
  <table class="table">
    <thead><tr><th>Name</th><th>Status</th><th></th></tr></thead>
    <tbody>
      <tr><td><a href="#/jobs/ID">Job name</a></td><td><span class="pill successful">Successful</span></td><td class="actions"><button class="btn compact" type="button">Edit</button></td></tr>
    </tbody>
  </table>
</div>
```

A list page puts its search in `.search`, optional filters in `.filter-row`, and the table in a card. Table headers are uppercase and muted; numbers use `.num`, narrow columns use `.tight`, and the last action column uses `.actions`. Paging keeps the current page in the address, as the job list does.

### Status pills

`pill(status)` renders a `.pill` span with the translated label. The status is also the class: `.pending`, `.running`, `.successful`, `.failed`, `.canceled`, `.skipped`, and `.warn`. The label carries the meaning; the color is a second signal.

```html
<span class="pill running">Running</span>
<span class="pill failed">Failed</span>
```

### Banners

Use `.banner` with `.good`, `.bad`, or `.warn`. A save confirmation is `.banner.good`. A failed request is `.banner.bad` at the top of the dialog body or the page. A persistent caveat is `.banner.warn`.

```html
<div class="banner good">Saved.</div>
<div class="banner bad">The name is already in use.</div>
<p class="banner warn">This environment uses a proxy.</p>
```

### Tabs

Page-level tabs use `.tabs` and `.tab-bar` above the panels. Tabs inside an overlay use `.dialog-tabs`, `.dialog-tab`, and `.dialog-panel`. The overlay helper sets `role="tablist"`, `role="tab"`, `role="tabpanel"`, `aria-selected`, and `tabindex`.

```html
<div class="spread tab-bar">
  <div class="tabs">
    <button class="btn pill primary" type="button">Output</button>
    <button class="btn pill" type="button">Parameters</button>
  </div>
</div>
```

### Overlay dialog

Use `.overlay` for the backdrop and `.overlay-card` for the panel. `openDialog` builds the rest: `.dialog`, `.dialog-head`, `.dialog-body`, `.dialog-actions`, `.dialog-buttons`, and `.dialog-danger`. `.dialog-footer` is the narrow-screen footer. `.dialog-close` is the `×` button. `.overlay-card.dialog` is 40 rem wide; `.overlay-card.dialog.wide` is 920 px.

```html
<div class="overlay">
  <div class="overlay-card dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
    <div class="dialog-head"><h2 id="dialog-title">Edit template</h2><button class="btn ghost dialog-close" type="button" aria-label="Close">×</button></div>
    <form class="dialog-form">
      <div class="dialog-body"></div>
      <div class="dialog-actions">
        <div class="dialog-danger"><button class="btn danger" type="button">Delete</button></div>
        <div class="dialog-buttons"><button class="btn" type="button">Cancel</button><button class="btn pill primary" type="submit">Save</button></div>
      </div>
    </form>
  </div>
</div>
```

### Logs and code

Use `.log` for job output, `.log-tools` for its search and controls, and `pre.vars` for structured data. `.log.wrap` and `.log-summary` are supported variants. Use `code` or `.mono` for inline identifiers. Keep the log text selectable and never put a catalog key in it.

```html
<div class="log-tools"><input class="search" type="search" aria-label="Search the log"></div>
<div class="log wrap"></div>
<pre class="vars">{}</pre>
```

## Page or overlay

A collection and a detail view stay pages. Create and edit forms open in an overlay, so the list keeps its place and the address can be shared. A long or tabbed form uses `openEditDialog`; a short form uses `openDialog`. A read-only detail that belongs to the current page can also be an overlay.

- Normal overlay: 40 rem (`openEditDialog` default, `openDialog` with `.dialog`).
- `openEditDialog` adds the `fields_marked_with_a_star_are_required` hint by default; pass `hint: ""` only when the form has no required field.
- Wide overlay: 920 px (`wide: true`), for the schedule and template forms.
- Address: `#/credentials?new=1`, `#/credentials?edit=<id>`, and `#/templates?edit=<id>&tab=target`. `?new=` accepts a kind such as `#/team?new=member`. The helpers are `editQuery`, `editAddress`, `rememberEditHash`, and `clearEditHash`. A tab change replaces the address with `replaceState`; opening and closing push or replace it as the helper does.
- Discard question: while the overlay is dirty, closing with `×`, Cancel, Escape, a backdrop click, a route change, or a team switch asks `discard_unsaved_changes` first. Do not add a second confirmation for the same action.
- Errors: `handle()` clears old banners and puts one `.banner.bad` at the top of the dialog body. It then marks the field with `.field-error`, sets `aria-invalid`, and focuses the control. In a tabbed dialog the tab also gets `.has-error` and `aria-invalid`, and the dialog switches to that tab.
- Focus: `makeModal` remembers the opener, focuses the first form control, keeps Tab inside the dialog, closes on Escape, and returns focus to the opener.

Something stays a page when it needs its own address, a deep link to a record, a long log, or a full workspace. Jobs, runs, workflow canvases, and project, inventory, and environment details are pages.

## Wording

- Write sentence case. A button is a verb: `Save`, `Create`, `Cancel`, `Delete`, `Edit`, `Launch`, `Search`.
- Use the same term for the same thing everywhere: `Credentials`, `Execution environment`, `Template`, `Workflow`, `Inventory`, `Project`, `Schedule`, `Job`, `Run`.
- Use `Create` when the object does not exist yet and `Save` when it does.
- API errors are keys. The UI translates them with `t(key, vars)`. Do not put English prose in a key, and do not put a key in a line that the job view shows raw.
- Avoid jargon in visible copy. Explain Docker, inventory, and Ansible terms in help text when they are unavoidable.
- Dates and times are in the user's local zone. `when()` uses the browser locale and a short zone name; schedules store and show local time.

## Accessibility

- Keep the global `:focus-visible` ring. A custom control must show the same ring, as `.chips` and `.switch` do.
- Every input has a label, either the wrapping `.field` label or `aria-label` for a search box.
- Dialogs use `role="dialog"`, `aria-modal="true"`, and `aria-labelledby`. Tabs support arrow keys, Home, and End. Escape closes a dialog and returns focus.
- Color is never the only signal. A status pill has text, a required field has `*`, and an error has text plus `aria-invalid`.
- Check contrast before you add a color pair. Use the existing token pairs instead of a new hex value.

## Responsive behavior

- At 900 px the sidebar becomes an off-canvas panel with `.nav-toggle` and `.scrim`; `.main` padding drops to 16 px. The schedule layout stacks, and `.dialog-footer` becomes visible.
- At 600 px `.overlay` loses its padding and `.overlay-card` becomes full screen.
- Tables scroll inside `.table-wrap`. Add a minimum width only to a table that needs it, as `.inventory-hosts-table` and `.schedule-table` do.
- Check every UI change at 1280 px and 390 px, in German and English.

## Pull request checklist

- [ ] The change uses the shared components and helpers instead of new markup.
- [ ] No component style is scoped to an id. Form controls use `.overlay-card.dialog` and `.field`, not a dialog id.
- [ ] New strings are keys in `web/i18n/de.json` and `web/i18n/en.json`, and the catalogs still match.
- [ ] The page was checked at 1280 px and 390 px, in German and English.
- [ ] The overlay or page follows the address, discard, error, and focus rules above.
- [ ] The pull request carries screenshots for a visible change.
- [ ] The guide and this checklist were read before the change.
