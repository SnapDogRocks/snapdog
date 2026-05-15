---
name: SnapDog
colors:
  background:
    light: "oklch(0.985 0.002 75)"
    dark: "oklch(0.147 0.004 49)"
  foreground:
    light: "oklch(0.147 0.004 49)"
    dark: "oklch(0.97 0.001 75)"
  card:
    light: "oklch(1 0 0)"
    dark: "oklch(0.216 0.006 56)"
  card-foreground:
    light: "oklch(0.147 0.004 49)"
    dark: "oklch(0.97 0.001 75)"
  primary: "oklch(0.769 0.188 70.08)"
  primary-foreground: "oklch(0.147 0.004 49)"
  secondary:
    light: "oklch(0.97 0.001 75)"
    dark: "oklch(0.268 0.006 58)"
  secondary-foreground:
    light: "oklch(0.268 0.006 58)"
    dark: "oklch(0.97 0.001 75)"
  muted:
    light: "oklch(0.97 0.001 75)"
    dark: "oklch(0.268 0.006 58)"
  muted-foreground:
    light: "oklch(0.553 0.013 58)"
    dark: "oklch(0.709 0.01 56)"
  border:
    light: "oklch(0.923 0.003 73)"
    dark: "oklch(1 0 0 / 10%)"
  destructive: "oklch(0.577 0.245 27.325)"
  ring: "oklch(0.769 0.188 70.08)"
  status-connected: "#22c55e"
  status-disconnected: "var(destructive)"
  eq-curve: "#f59e0b"
typography:
  sans:
    fontFamily: "ui-sans-serif, -apple-system, BlinkMacSystemFont, 'SF Pro Display', 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif"
  mono:
    fontFamily: "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace"
  body:
    fontSize: 14px
    fontWeight: "400"
    lineHeight: 20px
  label-sm:
    fontSize: 12px
    fontWeight: "500"
    lineHeight: 16px
  label-xs:
    fontSize: 10px
    fontWeight: "500"
    lineHeight: 14px
    letterSpacing: 0.05em
    textTransform: uppercase
  heading:
    fontSize: 16px
    fontWeight: "600"
    lineHeight: 24px
rounded:
  sm: 0.45rem
  DEFAULT: 0.6rem
  md: 0.75rem
  lg: 0.75rem
  xl: 1.05rem
  2xl: 1.35rem
  full: 9999px
spacing:
  base: 4px
  unit: 8px
  xs: 4px
  sm: 6px
  md: 12px
  lg: 16px
  xl: 24px
  section: 20px
breakpoints:
  sm: 576px
  md: 768px
  lg: 868px
  xl: 1280px
motion:
  duration-fast: 150ms
  duration-normal: 200ms
  duration-slow: 300ms
  easing: ease-in-out
  spring-stiffness: 300
  spring-damping: 30
  reduced-motion: "prefers-reduced-motion: reduce"
components:
  transport-button-primary:
    size: 48px
    rounded: "{rounded.full}"
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-foreground}"
  transport-button-ghost:
    size: 40px
    rounded: "{rounded.full}"
    backgroundColor: transparent
    textColor: "{colors.foreground}"
  client-chip:
    backgroundColor: "{colors.muted}"
    rounded: "{rounded.lg}"
    padding: "10px 12px"
    shadow: "inset 0 2px 4px rgba(0,0,0,0.15)"
    border: "1px solid var(border) / 50%"
  cover-art:
    rounded: "{rounded.2xl}"
    shadow: "0 10px 15px -3px rgba(0,0,0,0.1), 0 4px 6px -4px rgba(0,0,0,0.1)"
    aspect-ratio: "1 / 1"
  overlay-panel:
    rounded: "{rounded.2xl}"
    backgroundColor: "{colors.card}"
    border: "1px solid var(border)"
    shadow: "0 20px 25px -5px rgba(0,0,0,0.1), 0 8px 10px -6px rgba(0,0,0,0.1)"
    padding: "{spacing.xl}"
    backdrop: "rgba(0,0,0,0.5) blur(4px)"
  segmented-control:
    backgroundColor: "{colors.muted}"
    rounded: "{rounded.lg}"
    padding: 2px
    item-active-bg: "{colors.background}"
    item-active-shadow: "0 1px 2px rgba(0,0,0,0.05)"
    item-padding: "4px 12px"
    item-rounded: "{rounded.md}"
    item-fontSize: 12px
  slider-track:
    height: 12px
    rounded: "{rounded.full}"
    backgroundColor: "foreground / 20%"
  slider-thumb:
    size: 16px
    rounded: "{rounded.full}"
    backgroundColor: white
    border: "1px solid var(primary)"
    shadow: "0 1px 2px rgba(0,0,0,0.05)"
  zone-rail-item:
    rounded: "{rounded.lg}"
    padding: "12px"
    active-bg: "{colors.muted}"
  popover:
    rounded: "{rounded.xl}"
    backgroundColor: "var(popover) / 95%"
    backdrop-filter: "blur(24px)"
    border: "1px solid var(border) / 50%"
    shadow: "0 10px 15px -3px rgba(0,0,0,0.1)"
---

## Brand & Style

SnapDog's visual identity is that of a premium audio appliance — warm, confident, and understated. The design draws from high-end hi-fi equipment aesthetics: dark surfaces with warm amber accents, generous spacing, and tactile controls that feel physical. The brand personality is professional yet approachable, technical yet never cold.

The UI follows a "dark studio" philosophy in dark mode — like a mixing console in a dimly lit recording studio — while light mode presents a clean, warm stone palette reminiscent of Scandinavian industrial design. The amber primary color evokes vacuum tubes, warm analog sound, and the golden hour.

## Colors

The palette is built on warm stone neutrals with a single, saturated amber accent. This constraint creates visual calm in a multi-zone audio controller where information density is high.

- **Primary (Amber-500):** Used sparingly for active states, the play button, EQ curves, and status indicators. Never used for large surfaces.
- **Stone Neutrals:** A carefully tuned warm gray scale (not blue-gray) that feels organic and avoids the clinical coldness of pure grays.
- **Dark Mode Borders:** Use white at 10% opacity rather than a solid gray — this creates depth without hard edges and adapts naturally to adjacent surface colors.
- **Status Colors:** Green for connected clients, destructive red for disconnected. These are the only non-amber chromatic colors in the interface.

## Typography

The system font stack prioritizes SF Pro Display on Apple platforms and Segoe UI on Windows, ensuring the UI feels native to each operating system. No custom web fonts are loaded — this keeps the interface fast and consistent with OS conventions.

- **Hierarchy:** Achieved through weight and size rather than color. Body text is 14px, labels are 12px, and micro-labels (source badges, status pills) are 10px uppercase with wide tracking.
- **Truncation:** Long track titles and artist names use CSS truncation with a marquee animation on hover/focus for the currently playing track.
- **Tabular Numbers:** Volume percentages and time displays use tabular-nums to prevent layout shift during playback.

## Layout & Spacing

The layout is a responsive split-panel design optimized for both desktop monitoring and mobile control.

- **Mobile (< 768px):** Full-screen zone detail with swipe navigation between zones. Cover art dominates the viewport.
- **Tablet/Desktop (≥ 768px):** Fixed sidebar (224px) with zone rail + main content area showing the selected zone.
- **Wide Desktop (≥ 1280px):** Three-column layout with persistent zone list, zone detail, and expanded client/playlist panels.
- **Spacing Rhythm:** 4px base unit. Component internal padding uses 8-12px, section gaps use 16-20px. The interface is dense but never cramped.

## Elevation & Depth

Depth is communicated through surface color shifts and subtle inset shadows rather than dramatic drop shadows.

- **Cards:** Use the card surface color (white in light, stone-900 in dark) with no shadow on the card itself — the background contrast provides separation.
- **Client Chips:** Use an inset shadow (`inset 0 2px 4px rgba(0,0,0,0.15)`) to create a "pressed into the surface" tactile feel, reinforcing their draggable nature.
- **Overlays (EQ, About, API Key):** Full-screen backdrop blur with a centered panel using `shadow-xl` — the only place large shadows appear.
- **Popovers:** Use `backdrop-blur(24px)` with 95% opacity background for a frosted glass effect that maintains context.

## Shapes

The shape language is soft but not playful — rounded enough to feel modern, sharp enough to feel professional.

- **Transport Controls:** Fully circular (`rounded-full`) — these are the most tactile elements, mimicking physical knobs and buttons.
- **Cover Art:** Large radius (`rounded-2xl` on mobile, `rounded-xl` on desktop) — the hero visual element gets the softest treatment.
- **Cards & Panels:** Medium radius (`rounded-lg` to `rounded-xl`) — structural containers that frame content.
- **Inputs & Chips:** Smaller radius (`rounded-md` to `rounded-lg`) — functional elements that need to feel precise.
- **Segmented Controls:** The container uses `rounded-lg`, individual segments use `rounded-md` — creating a nested radius harmony.

## Motion

Animation is restrained and purposeful — it communicates state changes without drawing attention to itself.

- **Micro-interactions:** Transport buttons scale to 90% on tap (spring physics: stiffness 300, damping 30). Disabled when `prefers-reduced-motion` is set.
- **Transitions:** Color and opacity changes use 150-200ms ease-in-out. Never animate layout properties (width, height, padding).
- **Zone Switching (Mobile):** Horizontal swipe with spring-based snap. Dot indicators transition width and opacity.
- **Marquee:** Overflowing track titles animate with an 8s alternating ease-in-out, pausing at each end for readability.
- **Loading States:** Pulse animation on skeleton placeholders. Spin animation on connection status indicator.

## Components

### Transport Controls

A centered row of circular buttons: Previous, Play/Pause (larger, primary-colored), Next. The play/pause button supports long-press (600ms) to stop playback — no dedicated stop button exists. Ghost variant buttons for skip controls become disabled (50% opacity) when navigation is unavailable.

### Client Chips

Draggable cards with an inset shadow that communicates "grabbable." Each chip shows: connection dot (green/red), optional emoji icon, client name, volume slider, and a drag handle (dot grid). Chips can be dragged between zones to reassign clients.

### EQ Curve (Interactive)

An SVG-based parametric equalizer with draggable band nodes. Double-click adds a band, dragging a node past the vertical bounds deletes it (with opacity feedback at 30%). The curve renders a filled area under the frequency response line. Grid lines at standard frequencies (50Hz–10kHz) and dB levels (±12dB) provide reference.

### Segmented Controls

Used for EQ On/Off, Speaker mode (Off/Spinorama/Custom), and tab switching. A pill-shaped container with `rounded-lg`, containing items that highlight with a white/background fill and subtle shadow when active.

### Cover Art

Square aspect ratio with large border radius. Displays album art, radio station logos, or a branded placeholder SVG. Shadow provides lift from the surface. On mobile, cover art scales to fill available width; on desktop it's constrained to a fixed size alongside track metadata.

### Overlays

Full-viewport modal panels (EQ editor, About, API key prompt) use a semi-transparent backdrop with blur. The panel itself is a card-colored surface with `rounded-2xl`, `shadow-xl`, and generous padding. All overlays trap focus and dismiss on Escape.
