<script setup>
import { brands } from '../../brands.mjs';

const pick = (names) => names.map((name) => brands.find((brand) => brand.name === name));
// Agents scroll right to left, frameworks left to right.
const rows = [
  { key: 'agents', label: 'Coding agents', marks: pick(['Claude Code', 'Codex', 'Cursor', 'Copilot', 'Gemini CLI', 'OpenCode']) },
  { key: 'frameworks', label: 'Frameworks', marks: pick(['Rails', 'Django', 'Next.js', 'Vite', 'Phoenix', 'Laravel']) },
];
// Two identical halves move by half the track, so the loop has no seam. Each half repeats
// the set to stay wider than large screens. Only the first set of each row is read aloud.
const halves = [0, 1];
const rounds = [0, 1];
</script>

<template>
  <section class="stack" aria-labelledby="stack-heading">
    <p id="stack-heading" class="stack-label">Works with any coding agent that runs shell commands, and any app that starts with a command</p>
    <div v-for="row in rows" :key="row.key" class="marquee" :class="row.key">
      <div class="track">
        <ul
          v-for="half in halves"
          :key="half"
          class="stack-row"
          :class="{ copy: half > 0 }"
          :aria-label="half === 0 ? row.label : null"
          :aria-hidden="half > 0 ? 'true' : null"
        >
          <template v-for="round in rounds" :key="round">
            <li
              v-for="brand in row.marks"
              :key="`${round}-${brand.name}`"
              :class="{ repeat: round > 0 }"
              :aria-hidden="round > 0 ? 'true' : null"
              :style="{ '--brand': brand.color, '--brand-dark': brand.dark }"
            >
              <svg viewBox="0 0 24 24" aria-hidden="true"><path :d="brand.path" fill="currentColor" /></svg>
              {{ brand.name }}
            </li>
          </template>
        </ul>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* A full-width band in the secondary surface, like a payment-logo band. */
.stack {
  margin-top: 104px;
  padding: 40px 0 52px;
  background: var(--secondary);
  text-align: center;
}
.stack-label {
  margin: 0 0 28px;
  padding: 0 var(--home-gutter);
  color: var(--subtle-foreground);
  font-size: 15px;
}
.marquee {
  overflow: hidden;
  mask-image: linear-gradient(90deg, transparent, #000 10%, #000 90%, transparent);
}
.track {
  display: flex;
  width: max-content;
  animation: stack-scroll 70s linear infinite;
}
.marquee + .marquee {
  margin-top: 28px;
}
.frameworks .track {
  animation-direction: reverse;
}
.marquee:hover .track {
  animation-play-state: paused;
}
.stack-row {
  display: flex;
  flex: none;
  align-items: center;
  margin: 0;
  padding: 0;
  list-style: none;
}
li {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: 18px;
  padding: 6px clamp(32px, 4vw, 56px);
  color: var(--foreground);
  font-size: clamp(32px, 3.4vw, 46px);
  font-weight: 750;
  letter-spacing: -0.035em;
  white-space: nowrap;
}
svg {
  flex: none;
  width: 1.15em;
  height: 1.15em;
  color: var(--brand);
}
:root.dark svg {
  color: var(--brand-dark);
}
@keyframes stack-scroll {
  to { transform: translateX(-50%); }
}
/* Without motion, one centered set wraps in place. */
@media (prefers-reduced-motion: reduce) {
  .marquee {
    mask-image: none;
  }
  .track {
    width: auto;
    justify-content: center;
    animation: none;
  }
  .stack-row {
    flex: 1;
    flex-wrap: wrap;
    justify-content: center;
    gap: 16px 0;
    padding: 0 var(--home-gutter);
  }
  .copy,
  .repeat {
    display: none;
  }
  li {
    padding: 4px 20px;
    font-size: 24px;
  }
}
@media (max-width: 699px) {
  .stack {
    margin-top: 80px;
    padding: 32px 0 40px;
  }
  li {
    gap: 12px;
    padding: 4px 24px;
    font-size: 28px;
  }
}
</style>
