# Parcela brand assets

Fence Buddy is the main logo: three ivory pickets with two charcoal eyes and no marks above the eyes. The site uses the transparent PNG.

| File | Purpose |
| --- | --- |
| [`logo.png`](logo.png) | The 512×512 site logo, resized from the generated Fence Buddy master with transparency preserved. |
| [`../../src/dashboard/badge.png`](../../src/dashboard/badge.png) | The icon of the debug menu's badge, built from the same master. |
| [`fence-buddy-reference.png`](fence-buddy-reference.png) | Unmodified imagegen output of the selected ivory mascot with the two board marks removed. |
| [`fence-buddy-prompt.json`](fence-buddy-prompt.json) | Exact imagegen edit prompt and source reference. |

Fence Buddy uses an ivory body and charcoal eyes and outline. It has no blue accent or separate face panel.

## Rebuild the site logo and badge

Install the docs dependencies, then resize the generated master:

```sh
npm ci --prefix docs
node scripts/build-brand-icon.mjs
```

The builder removes 10% of transparent padding on each edge and uses resvg to resize the PNG. It does not redraw the mascot or convert it to vector paths.

## Source history

Fence Buddy and its final edit came from the built-in imagegen tool. The concepts and their prompts are in `concepts/`.
