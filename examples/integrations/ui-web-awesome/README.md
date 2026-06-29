# ui-web-awesome — Web Awesome (Shoelace v3) UI components

> **Different category — NOT a CO-503 tool.** There is no manifest here. This
> sample lives under `examples/integrations/` because it demonstrates a *frontend*
> integration path, the UI analogue of "plug in OSS without a rewrite". It does
> not register with `CanonicalToolRegistry` and is unrelated to
> `CO_ENABLE_EXTERNAL_TOOLS`.

**What it demonstrates.** A **Web Awesome** component (Shoelace v3, by the Font
Awesome team) dropped into a vanilla-TS / plain-HTML page via the **autoloader
CDN** — proving the **framework-agnostic, no-rewrite UI path**. Custom elements
(`<wa-button>`, `<wa-input>`, `<wa-dialog>`) are Web Components: they work the
same in plain HTML, in the existing SvelteKit SPA, or anywhere else, with **no
build step and no npm install**. The autoloader registers each element on first
use straight from the CDN.

## AS-IS vs TO-BE

| | |
|---|---|
| **AS-IS** | Each UI control is hand-rolled — bespoke `<button>`/markup plus its own CSS, and modal/focus-trap/validation logic written by hand. Every widget is maintenance. |
| **TO-BE** | One **custom element** per control (`<wa-button variant="brand">`, `<wa-dialog>`, `<wa-input>`). Behaviour (focus trapping, validation, sizing, a11y) ships inside the element. `index.html` shows both side by side on the same page. |

## How it coexists with the existing SPA

- Web Components are **standard DOM** — the existing SvelteKit SPA can render
  `<wa-*>` tags directly; Svelte treats unknown elements as custom elements with
  no special bindings required.
- Migration is **incremental**: replace one control at a time. The sample keeps a
  hand-rolled `.legacy-btn` next to a `<wa-button>` to make the point — both share
  the same DOM and can share the same event handlers.
- The two CDN tags (`webawesome.css` + `webawesome.loader.js`) are added once to
  the page shell; no bundler change.

## Theming — CSS variables, no Tailwind

Theming is **plain CSS custom properties**, no utility classes and no Tailwind.
Overriding a handful of Web Awesome design tokens on `:root` re-skins the entire
component set to CO's palette:

```css
:root {
  --wa-color-brand-fill-loud: #3b5bdb;   /* CO accent */
  --wa-color-brand-on-loud:   #ffffff;
}
```

## Try it

```bash
# any static server; e.g.
python3 -m http.server -d examples/integrations/ui-web-awesome 8080
# then open http://localhost:8080  (needs internet for the CDN autoloader)
```

> The CDN URLs pin `webawesome@3.0.0-beta.4` via the `early.webawesome.com` beta
> channel. Confirm/refresh the version against the current beta before adopting.
