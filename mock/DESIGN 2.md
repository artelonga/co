# Design System Document: Cinematic RPG Editorial

## 1. Overview & Creative North Star
**Creative North Star: "The Relic Archive"**

This design system moves away from the sterile, "app-like" interfaces of modern productivity tools and instead embraces the gravity of a high-fantasy digital manuscript. We are not just organizing data; we are curating a saga. The aesthetic breaks the "template" look through **intentional asymmetry**, where large editorial display type balances against dense, functional data columns. By utilizing deep tonal layering and ethereal glows, we create a sense of infinite depth—as if the user is looking into a pool of midnight ink where golden truths reside.

## 2. Colors & Surface Philosophy
The palette is rooted in a moody, low-light environment. We treat the screen not as a flat canvas, but as a dimly lit chamber.

### Surface Hierarchy & Nesting
To achieve "The Relic Archive" look, we forbid the use of traditional 1px solid borders for sectioning.
- **The "No-Line" Rule:** Boundaries are defined exclusively through background shifts. Place a `surface_container_low` card on a `surface` background to create a soft, natural lift.
- **Nesting Logic:** 
    - Base Level: `surface` (#131313)
    - Sectioning: `surface_container_low` (#1c1b1b)
    - Interactive Cards: `surface_container` (#201f1f)
    - Elevated Modals/Popovers: `surface_container_highest` (#353534)

### The "Glass & Gradient" Rule
Standard flat colors feel "cheap" in an epic RPG context. 
- **Backdrop Blur:** Floating elements (tooltips, navigation bars) must use `surface_container` at 80% opacity with a `20px` backdrop-blur. This allows the "midnight blue" and "burgundy" tones of the background content to bleed through.
- **Signature Gradients:** For high-drama CTAs, use a subtle linear gradient from `primary` (#ffb3b5) to `on_primary_container` (#e0505f) at a 45-degree angle. This provides a "blood-silk" texture that flat hex codes cannot replicate.

## 3. Typography
Our typography is a dialogue between the prestigious past and the functional present.

*   **Display & Headlines (Newsreader):** Use this serif for storytelling. It conveys the "epic" nature of the RPG. Utilize `display-lg` for character names or chapter titles to create a high-contrast editorial feel.
*   **Titles & Body (Manrope):** A clean, modern sans-serif used for technical data, stats, and long-form descriptions. The juxtaposition of a traditional serif with a technical sans-serif creates a "Digital Scholar" aesthetic.
*   **The "Prestigious" Scale:** Always lean into the extremes. Use `display-lg` (3.5rem) next to `label-sm` (0.68rem) to create visual drama through scale.

## 4. Elevation & Depth
In this system, light doesn't come from a generic top-down source; it glows from within the elements themselves.

- **The Layering Principle:** Depth is achieved by "stacking" the surface-container tiers. Never use a shadow to separate a card from a background if a tonal shift can do the work.
- **Ambient Shadows:** When an element must float (e.g., a high-level inventory modal), use an ultra-diffused shadow: `0 20px 50px rgba(0, 0, 0, 0.5)`. The shadow should feel like a "presence" rather than a line.
- **The "Ghost Border" Fallback:** If accessibility requires a container edge, use `outline_variant` at **15% opacity**. This creates a "whisper" of a boundary that doesn't break the cinematic immersion.
- **Ethereal Accents:** Use `secondary` (#e9c349) for active states. It should feel like a "Glowing Gold" thread running through the charcoal and burgundy.

## 5. Components

### Buttons
- **Primary:** Gradient from `primary` to `on_primary_container`. Text in `on_primary`. High-end "silk" feel.
- **Secondary:** Transparent background with a `Ghost Border` (15% `outline_variant`). On hover, the border opacity increases to 40%.
- **Tertiary:** Text-only using `secondary` (Gold). Reserved for "Ethereal" actions like "Reveal Secrets" or "Undo."

### Cards & Lists
- **The "No-Divider" Mandate:** Forbid horizontal lines between list items. Use a `1.5` (0.5rem) spacing gap and a subtle background shift (`surface_container_low`) on hover to indicate selection.
- **Editorial Cards:** Combine a `headline-sm` title with a `body-sm` description. Use asymmetrical padding (e.g., more padding on the left than the right) to create a custom, non-bootstrap look.

### Input Fields
- **Styling:** Use `surface_container_lowest` for the input track. No bottom border.
- **Focus State:** Instead of a thick blue ring, use a subtle 1px glow using `secondary` (Gold) at 50% opacity and a 4px outer blur.

### RPG-Specific Components
- **Character Stat Hexes:** Use `surface_variant` shapes with `tertiary` text.
- **Status Badges:** Small, pill-shaped `chips` using `primary_container` backgrounds with `primary` text for a "wounded/burgundy" look, or `secondary_container` for "buffs/gold."

## 6. Do's and Don'ts

### Do
- **Do** use `20` (7rem) or `24` (8.5rem) spacing for major section breathing room. Drama requires space.
- **Do** use `Newsreader` for any text that is meant to be "read" like a story.
- **Do** use `Manrope` for "meta" information (UI labels, timestamps, button text).
- **Do** treat "Ethereal White" (`on_surface`) as a precious resource; use it for primary content only.

### Don't
- **Don't** use 100% opaque white for secondary text; use `on_surface_variant` (#c5c6cc) to maintain the moody atmosphere.
- **Don't** use sharp corners. Stick to the `md` (0.375rem) or `lg` (0.5rem) roundedness to keep the aesthetic "sophisticated" rather than "brutalist."
- **Don't** use standard "Success Green." If something is successful, use the "Glowing Gold" (`secondary`) to indicate a "legendary" result.
- **Don't** clutter the screen. If a piece of info isn't vital to the "saga," hide it in a `surface_container` popover.