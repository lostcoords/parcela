<script setup>
import { withBase } from 'vitepress';
import { ArrowRight } from '@lucide/vue';
import { Button } from '@/components/ui/button';
import desktop from '@/media/app-desktop.jpg';
import phone from '@/media/app-phone.jpg';
import DeviceFrame from './DeviceFrame.vue';
import SectionHeading from './SectionHeading.vue';
</script>

<template>
  <section class="home-section steps" aria-labelledby="setup">
    <div class="section-head center">
      <SectionHeading id="setup">
        Set up your project once
      </SectionHeading>
      <p class="section-lead">
        Parcela starts the commands you list in <code>parcela.yaml</code>. Your setup command
        installs dependencies and prepares data, as it does today.
      </p>
      <div class="section-actions">
        <Button as-child><a :href="withBase('/docs/getting-started/')">Set up your app <ArrowRight aria-hidden="true" /></a></Button>
        <Button as-child variant="secondary"><a :href="withBase('/docs/tutorial/')">Try the example app</a></Button>
      </div>
    </div>

    <!-- Describe and start side by side; the running app spans the row below. -->
    <ol class="bento">
      <li class="step tile describe">
        <div class="copy">
          <span class="step-number">01</span>
          <h3>Describe your app</h3>
          <p>
            Run <code class="inline-code">parcela init</code>, then review the generated
            <code class="inline-code">parcela.yaml</code>.
          </p>
        </div>
        <div class="visual editor">
          <span class="file">parcela.yaml</span>
          <pre class="code"><span class="k">project</span>: myapp
<span class="k">services</span>:
  <span class="k">web</span>:
    <span class="k">command</span>: bin/rails server -p <span class="v">$PORT</span>
    <span class="k">http</span>:
      <span class="k">healthcheck</span>: /up
  <span class="k">css</span>:
    <span class="k">command</span>: bin/rails tailwindcss:watch[always]
<span class="k">hooks</span>:
  <span class="k">setup</span>:
    <span class="k">command</span>: bin/setup --skip-server</pre>
        </div>
      </li>

      <li class="step tile start">
        <div class="copy">
          <span class="step-number">02</span>
          <h3>Start the app</h3>
          <p>
            <code class="inline-code">parcela up</code> runs setup on the first start, then starts each
            service and waits for its healthcheck.
          </p>
        </div>
        <div class="visual terminal">
          <span class="file">~/myapp</span>
          <pre class="code wrap"><span class="p">$</span> parcela up --detach
<span class="ok">✓</span> myapp is running at <span class="v">https://myapp.localhost</span>

<span class="o">  parcela logs    show service output
  parcela status  check health
  parcela down    stop it</span></pre>
        </div>
      </li>

      <li class="step tile open">
        <div class="open-text">
          <div class="copy">
            <span class="step-number">03</span>
            <h3>Open its URL</h3>
            <p>
              The project name becomes the URL. Other worktrees get their own, like
              <code class="inline-code">receipts-myapp.localhost</code>. With
              <code class="inline-code">--lan</code>, your phone opens it at
              <code class="inline-code">myapp.local</code>.
            </p>
          </div>
        </div>
        <div class="pair">
          <DeviceFrame class="pair-laptop" :src="desktop" url="https://myapp.localhost" alt="The app's homepage at its local HTTPS address, on a laptop" />
          <DeviceFrame class="pair-phone" kind="phone" :src="phone" url="https://myapp.local" alt="The same page on a phone" />
        </div>
      </li>
    </ol>
  </section>
</template>

<style scoped>
.bento {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 20px;
  margin: 56px 0 0;
  padding: 0;
  list-style: none;
}
.tile {
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
  border-radius: 24px;
  background: var(--secondary);
}
.open {
  display: grid;
  grid-column: 1 / -1;
  grid-template-columns: minmax(0, 0.8fr) minmax(0, 1.2fr);
  align-items: start;
}
.open-text {
  padding-bottom: 28px;
}
.copy {
  padding: 28px 28px 0;
}
.step-number {
  display: block;
  color: var(--subtle-foreground);
  font-family: var(--vp-font-family-mono);
  font-size: 12px;
  font-weight: 600;
}
h3 {
  margin: 6px 0 0;
  font-size: 19px;
  font-weight: 750;
  letter-spacing: -0.02em;
}
.copy p {
  margin: 8px 0 0;
  color: var(--muted-foreground);
  font-size: 15px;
  line-height: 1.7;
}
/* The file reads as an editor and the command as a terminal, each on its own surface inside
   the tile. The terminal stays dark in both themes. */
.visual {
  /* Both top surfaces grow to the same height. */
  flex: 1;
  margin: 20px;
  padding: 16px 18px 18px;
  border-radius: 14px;
}
.editor {
  background: var(--background);
}
.terminal {
  background: #16181c;
  color: #e4e6ea;
}
:root.dark .terminal {
  background: #0c0d0f;
}

.file {
  display: block;
  margin-bottom: 10px;
  color: var(--subtle-foreground);
  font-family: var(--vp-font-family-mono);
  font-size: 11px;
}
.code {
  margin: 0;
  overflow: hidden;
  color: var(--foreground);
  font-family: var(--vp-font-family-mono);
  font-size: 12px;
  line-height: 1.6;
  white-space: pre;
}
.code.wrap {
  white-space: pre-wrap;
}
.code .k,
.code .ok {
  color: #287547;
}
.code .v,
.code .p {
  color: var(--home-sky);
}
.code .o {
  color: var(--subtle-foreground);
}
.terminal .file,
.terminal .code .o {
  color: #8b9099;
}
.terminal .code {
  color: #e4e6ea;
}
.terminal .code .ok {
  color: #79ca94;
}
.terminal .code .v,
.terminal .code .p {
  color: #65c6e3;
}
:root.dark .code .k,
:root.dark .code .ok {
  color: #79ca94;
}
:root.dark .code .v,
:root.dark .code .p {
  color: #65c6e3;
}
/* The laptop with a phone in front of its right corner. */
.pair {
  position: relative;
  margin: 36px 40px 36px 12px;
  padding-bottom: 12%;
}
.pair-laptop {
  width: 80%;
  /* Room for the base, which is wider than the lid. */
  margin-left: 6%;
}
.pair-phone {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 25%;
}

@media (max-width: 959px) {
  .bento {
    grid-template-columns: minmax(0, 1fr);
    max-width: 560px;
    margin-inline: auto;
  }
  .open {
    grid-template-columns: minmax(0, 1fr);
  }
  .open-text {
    padding-bottom: 0;
  }
  .pair {
    margin: 24px 28px 28px 20px;
  }
}
@media (max-width: 699px) {
  .bento {
    margin-top: 40px;
    gap: 16px;
  }
  .tile {
    border-radius: 20px;
  }
  .copy {
    padding: 22px 20px 0;
  }
  .visual,
  .pair {
    margin-inline: 14px;
  }
  .visual {
    padding: 14px;
  }
  .code {
    font-size: 11px;
    white-space: pre-wrap;
  }
}
</style>
