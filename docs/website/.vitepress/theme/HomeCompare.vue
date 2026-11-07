<script setup>
import { ref } from 'vue';
import { withBase } from 'vitepress';
import { ArrowRight, Check, X } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import { agents } from './agents.js';
import CodeText from './CodeText.vue';
import PenMark from './PenMark.vue';
import SectionHeading from './SectionHeading.vue';
import { useEntrance } from './useHomeMotion.js';

const agent = (name) => agents.find((item) => item.name === name);
const worktrees = [
  { name: 'receipts', agent: agent('Claude Code') },
  { name: 'settle-up', agent: agent('Codex') },
  { name: 'group-stats', agent: agent('Cursor') },
];
const sides = [
  {
    key: 'without',
    title: 'Without Parcela',
    text: 'The first `bin/dev` takes port 3000. The others fail until it stops.',
    command: 'bin/dev',
    results: worktrees.map((worktree, i) => i === 0
      ? { ok: true, text: 'Listening on localhost:3000' }
      : { ok: false, text: 'Port 3000 is already in use' }),
  },
  {
    key: 'with',
    title: 'With Parcela',
    text: '`parcela up` picks free ports for each worktree and gives it an HTTPS name.',
    command: 'parcela up',
    results: worktrees.map((worktree) => ({ ok: true, text: `https://${worktree.name}-myapp.localhost` })),
  },
];

// The lines type in once the section scrolls into view.
const root = ref(null);
const { armed, playing } = useEntrance(root);
</script>

<template>
  <section ref="root" class="home-section compare" :class="{ armed, playing }" aria-labelledby="worktrees">
    <div class="section-head center">
      <SectionHeading id="worktrees">
        Each worktree runs <PenMark>its own copy</PenMark> of the app
      </SectionHeading>
      <p class="section-lead">
        Checkouts of the same app usually compete for one port, so only one agent at a time
        can run its changes. Parcela gives each checkout its own ports and URL, so every agent
        can open its own app and check its work.
      </p>
      <div class="section-actions">
        <Button as-child><a :href="withBase('/docs/worktrees/')">Run parallel worktrees <ArrowRight aria-hidden="true" /></a></Button>
      </div>
    </div>
    <div class="sides">
      <div v-for="(side, s) in sides" :key="side.key" class="side" :class="side.key">
        <div class="window">
          <div class="window-bar">~/myapp · 3 worktrees</div>
          <ol class="terminal">
            <li v-for="(worktree, i) in worktrees" :key="worktree.name">
              <span class="line command" :style="{ '--delay': `${(s * 3 + i) * 700}ms` }"
                ><svg viewBox="0 0 24 24" aria-hidden="true" :style="{ color: worktree.agent.color ?? 'currentColor' }"><path :d="worktree.agent.path" fill="currentColor" /></svg
                ><span class="prompt">{{ worktree.name }} <span class="dollar">$</span></span> {{ side.command }}</span
              >
              <span
                class="line result"
                :class="{ fail: !side.results[i].ok }"
                :style="{ '--delay': `${(s * 3 + i) * 700 + 520}ms` }"
                ><component :is="side.results[i].ok ? Check : X" aria-hidden="true" />{{ side.results[i].text }}</span
              >
            </li>
          </ol>
        </div>
        <h3>{{ side.title }}</h3>
        <p><CodeText :text="side.text" /></p>
      </div>
    </div>
  </section>
</template>

<style scoped>
.sides {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 28px;
  margin-top: 56px;
}
.terminal {
  display: grid;
  gap: 14px;
  margin: 0;
  padding: 18px 20px 20px;
  list-style: none;
  font-family: var(--vp-font-family-mono);
  font-size: 12.5px;
  line-height: 1.6;
}
.line {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
}
.command {
  color: var(--foreground);
}
.command svg {
  flex: none;
  width: 14px;
  height: 14px;
}
.prompt {
  color: var(--subtle-foreground);
}
.dollar {
  color: #65c6e3;
}
.result {
  padding-left: 22px;
  color: #79ca94;
}
.result svg {
  flex: none;
  width: 13px;
  height: 13px;
  stroke-width: 2.6;
}
.result.fail {
  color: var(--destructive);
}
h3 {
  display: block;
  margin: 24px 0 0;
  font-size: 19px;
  font-weight: 750;
  letter-spacing: -0.02em;
}
.side > p {
  margin: 8px 0 0;
  color: var(--muted-foreground);
  font-size: 15px;
  line-height: 1.7;
}

/* Commands type in from the left, then their result rises in. Lines hide instantly when
   armed and only transition once playing, so arming never starts a delayed fade. */
.armed:not(.playing) .command {
  clip-path: inset(0 100% 0 0);
}
.armed:not(.playing) .result {
  opacity: 0;
  transform: translateY(4px);
}
.playing .line {
  transition:
    clip-path 0.5s steps(14, end),
    opacity 0.3s ease,
    transform 0.3s ease;
  transition-delay: var(--delay);
}
.playing .command {
  clip-path: inset(0 0 0 0);
}

@media (max-width: 959px) {
  .sides {
    grid-template-columns: minmax(0, 1fr);
    max-width: 600px;
    margin-inline: auto;
    gap: 48px;
  }
}
@media (max-width: 699px) {
  .sides {
    margin-top: 40px;
  }
  .terminal {
    padding: 14px;
    font-size: 11px;
  }
  .result {
    padding-left: 0;
  }
  .result,
  .command {
    text-overflow: ellipsis;
  }
}
</style>
