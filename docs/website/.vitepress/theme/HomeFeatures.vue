<script setup>
import { withBase } from 'vitepress';
import { Button } from '@/components/ui/button';
import { Activity, ArrowRight, Bot, DatabaseBackup, Lock, Plug, ScrollText } from '@lucide/vue';
import CodeText from './CodeText.vue';
import SectionHeading from './SectionHeading.vue';

const capabilities = [
  {
    icon: Lock,
    title: 'Named HTTPS',
    text: 'Caddy serves each environment at a `.localhost` name with a local certificate, on a port that needs no sudo.',
    href: '/docs/faq/',
  },
  {
    icon: Plug,
    title: 'Free ports',
    text: 'Each HTTP service gets its own `$PORT`. Databases and other tools can ask for named ports. Running environments never share a port.',
    href: '/docs/configuration/#ports',
  },
  {
    icon: Activity,
    title: 'Readiness checks',
    text: '`parcela up` waits until every healthcheck returns 2xx before it routes the URLs.',
    href: '/docs/configuration/#http-services',
  },
  {
    icon: ScrollText,
    title: 'Logs per process',
    text: '`parcela logs -f` follows every service, worker, and hook, with the process name on each line.',
    href: '/docs/cli/#logs',
  },
  {
    icon: DatabaseBackup,
    title: 'Data snapshots',
    text: 'Snapshot the data of a stopped environment, then restore it into a new worktree to reproduce a test case.',
    href: '/docs/worktrees/#reproduce-a-test-dataset',
  },
  {
    icon: Bot,
    title: 'Agent skill',
    text: '`parcela --skill` prints the instructions a coding agent needs to run Parcela.',
    href: '/docs/worktrees/#set-up-a-coding-agent',
  },
];
</script>

<template>
  <section class="home-section capabilities" aria-labelledby="features">
    <div class="section-head">
      <SectionHeading id="features">
        What Parcela sets up for each environment
      </SectionHeading>
      <div class="section-actions">
        <Button as-child><a :href="withBase('/docs/configuration/')">Read the configuration reference <ArrowRight aria-hidden="true" /></a></Button>
      </div>
    </div>
    <ul class="capability-grid">
      <li v-for="item in capabilities" :key="item.title" class="capability">
        <a :href="withBase(item.href)">
          <component :is="item.icon" aria-hidden="true" />
          <h3>{{ item.title }}</h3>
          <p><CodeText :text="item.text" /></p>
        </a>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.capability-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px 28px;
  margin: 48px 0 0;
  padding: 0;
  list-style: none;
}
.capability a {
  display: block;
  height: 100%;
  margin: 0 -20px;
  padding: 22px 20px;
  border-radius: 18px;
  color: inherit;
  text-decoration: none;
  transition: background-color var(--motion-control) ease;
}
.capability a:hover {
  background: var(--secondary);
}
.capability svg {
  width: 22px;
  height: 22px;
  color: #65c6e3;
  stroke-width: 1.8;
}
h3 {
  margin: 16px 0 0;
  font-size: 17px;
  font-weight: 750;
  letter-spacing: -0.015em;
}
p {
  margin: 6px 0 0;
  color: var(--muted-foreground);
  font-size: 15px;
  line-height: 1.65;
}
@media (max-width: 959px) {
  .capability-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@media (max-width: 599px) {
  .capability-grid {
    grid-template-columns: minmax(0, 1fr);
    margin-top: 32px;
  }
  .capability a {
    padding: 18px 20px;
  }
}
</style>
