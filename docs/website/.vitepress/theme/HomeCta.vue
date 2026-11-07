<script setup>
import { withBase } from 'vitepress';
import { ArrowRight, Plus } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import InstallCommand from './InstallCommand.vue';
import SectionHeading from './SectionHeading.vue';

const questions = [
  {
    question: 'Does it work with my project?',
    answer:
      'Parcela runs the commands you put in parcela.yaml. Use it with native web servers, workers, and setup scripts. It supports macOS and Linux, with jj workspaces or Git worktrees.',
  },
  {
    question: 'How separate are the environments?',
    answer:
      'Each checkout gets its own processes, ports, and URLs. Your project configuration decides how databases, queues, and uploads are separated. Services run as native processes on your machine.',
  },
  {
    question: 'How does local HTTPS work?',
    answer:
      'Caddy handles routing and TLS. The default address uses .localhost on port 1443. Your first terminal up offers to trust the local CA behind the system password prompt. Scripts and agents never change system trust.',
  },
  {
    question: 'What happens when I stop an environment?',
    answer:
      "parcela down stops the environment's processes and removes its routes. Source and data are retained. LAN and Tailscale sharing stop with that run.",
  },
];
</script>

<template>
  <div class="home-closing">
    <section class="home-section questions" aria-labelledby="faq">
      <div class="questions-intro">
        <SectionHeading id="faq">Common questions</SectionHeading>
        <div class="section-actions">
          <Button as-child variant="secondary"><a :href="withBase('/docs/faq/')">Read the FAQ <ArrowRight aria-hidden="true" /></a></Button>
        </div>
      </div>
      <div class="question-list">
        <details v-for="item in questions" :key="item.question">
          <summary>{{ item.question }}<Plus aria-hidden="true" /></summary>
          <p>{{ item.answer }}</p>
        </details>
      </div>
    </section>
    <section class="home-section cta" aria-labelledby="install">
      <div class="cta-inner">
        <div class="pitch">
          <div class="mascot-reveal"><img class="mascot" :src="withBase('/assets/logo.png')" alt="" width="120" height="120" /></div>
          <SectionHeading id="install">Install Parcela</SectionHeading>
          <p class="cta-lead">Install the CLI, then run <code class="inline-code">parcela init</code> in your project.</p>
          <InstallCommand class="install" />
          <div class="cta-actions">
            <Button as-child size="lg" class="home-start">
              <a :href="withBase('/docs/getting-started/')">Get started <ArrowRight aria-hidden="true" /></a>
            </Button>
            <Button as-child variant="secondary" size="lg"><a :href="withBase('/docs/')">Read the docs</a></Button>
          </div>
          <p class="project-note">Experimental · macOS &amp; Linux · AGPL-3.0-only</p>
          <p class="telemetry-note">
            Anonymous command telemetry is on by default.
            <a :href="withBase('/docs/telemetry/')">See what is collected and how to turn it off.</a>
          </p>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.home-closing {
  width: 100%;
}
.questions {
  display: grid;
  grid-template-columns: minmax(0, 0.8fr) minmax(0, 1.2fr);
  gap: 72px;
}
.question-list {
  margin-top: -6px;
}
details {
  box-shadow: inset 0 -1px var(--border);
}
summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 18px;
  padding: 22px 0;
  font-size: 16px;
  font-weight: 700;
  list-style: none;
  cursor: pointer;
}
summary::-webkit-details-marker {
  display: none;
}
summary svg {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--subtle-foreground);
  transition: transform 0.2s ease;
}
details[open] summary svg {
  transform: rotate(45deg);
}
details > p {
  margin: -6px 40px 22px 0;
  color: var(--muted-foreground);
  font-size: 15px;
  line-height: 1.75;
}
.cta-inner {
  padding: 24px 0 0;
  text-align: center;
}
.pitch {
  max-width: 620px;
  margin: 0 auto;
}
.mascot-reveal {
  display: flex;
  justify-content: center;
  margin-bottom: 20px;
}
.mascot {
  width: 120px;
  height: 120px;
  transform-origin: 50% 90%;
}
.cta-lead {
  margin: 14px 0 0;
  color: var(--muted-foreground);
  font-size: 17px;
}
.cta .install {
  max-width: 480px;
  margin: 32px auto 0;
}
.cta-actions {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-wrap: wrap;
  gap: 16px 28px;
  margin-top: 24px;
}
.pitch > .project-note {
  margin: 28px 0 0;
  color: var(--subtle-foreground);
  font-size: 13px;
}
.pitch > .telemetry-note {
  margin: 6px 0 0;
  color: var(--subtle-foreground);
  font-size: 12px;
}
.telemetry-note a {
  color: inherit;
  text-decoration: underline;
  text-underline-offset: 3px;
}
@media (max-width: 819px) {
  .questions {
    grid-template-columns: minmax(0, 1fr);
    gap: 24px;
  }
}
@media (max-width: 699px) {
  .cta-inner {
    padding: 8px 0 0;
  }
  .mascot {
    width: 96px;
    height: 96px;
  }
  .install :deep(code) {
    padding: 0 42px 0 12px;
    font-size: clamp(9px, 2.5vw, 12px);
    letter-spacing: -0.03em;
  }
}
@media (prefers-reduced-motion: no-preference) {
  .mascot { animation: sway 4s ease-in-out 0.8s infinite; }
}
@keyframes sway {
  0%, 70%, 100% { rotate: 0deg; }
  78% { rotate: -6deg; }
  88% { rotate: 5deg; }
}
</style>
