# Parcela brand assets

Fence Buddy is the main logo: three ivory pickets with two charcoal eyes and no marks above the eyes. The site uses the transparent PNG.

| File | Purpose |
| --- | --- |
| [`logo.png`](logo.png) | The 512×512 site logo, resized from the generated Fence Buddy master with transparency preserved. |
| [`fence-buddy-reference.png`](fence-buddy-reference.png) | Unmodified imagegen output of the selected ivory mascot with the two board marks removed. |
| [`fence-buddy-prompt.json`](fence-buddy-prompt.json) | Exact imagegen edit prompt and source reference. |
| [`logo.svg`](logo.svg) | Previous waving picket mascot, retained as a source asset. |
| [`mascot-pixel.png`](mascot-pixel.png) | Pixel 01, the steady-wave mascot. A transparent 32×32 PNG with a one-pixel dark outline. |
| [`mascot-pixel.json`](mascot-pixel.json) | Editable pixel grid and exact RGBA palette. The builder adds the outline without changing the original foreground pixels. |
| [`mascot-reference.png`](mascot-reference.png) | Unmodified native image-generation output A, retained as the master reference. This raster has an opaque background and is not the reusable logo. |

Fence Buddy uses an ivory body and charcoal eyes and outline. It has no blue accent or separate face panel.

## Rebuild the site logo

Install the docs dependencies, then resize the generated master:

```sh
npm ci --prefix docs/website
node scripts/build-brand-icon.mjs
```

The builder removes 10% of transparent padding on each edge and uses resvg to resize the PNG. It does not redraw the mascot or convert it to vector paths.

## Rebuild the pixel mascot

Install the docs dependencies and run:

```sh
npm ci --prefix docs/website
node scripts/build-brand-pixel.mjs
```

## Display the assets

Display the pixel mark at its native size or an integer multiple. Use nearest-neighbor scaling, or `image-rendering: pixelated` in CSS. Do not use smoothing or fractional scaling when preserving the pixel grid matters.

## Source history

Fence Buddy and its final edit came from the built-in imagegen tool. The concepts and their prompts are in `concepts/`.

The previous SVG is a vector cleanup of the original generated picket concept. The pixel design is grid-authored. The original `mascot-reference.png` has SHA-256 `62373f4ec80f39a9814bad3c8ab6789c7fd7a5c0f5501353c1b4f5758c8cfd74`.
