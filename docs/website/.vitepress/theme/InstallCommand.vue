<script setup>
// The install command as VitePress's code block markup, so its copy button works the same.
const install = [
  ['#B392F0', 'curl'],
  ['#79B8FF', ' -fsSL'],
  ['#9ECBFF', ' https://parcela.run/install.sh'],
  ['#F97583', ' |'],
  ['#B392F0', ' sh'],
];
</script>

<template>
  <div class="vp-doc install">
    <div class="language-sh vp-adaptive-theme">
      <button title="Copy Code" class="copy"></button>
      <!-- Hidden like every label, but VitePress's copy button finds the code past it. -->
      <span class="lang">sh</span>
      <pre class="shiki github-dark vp-code" tabindex="0"><code><span class="line"><span v-for="([color, text], i) in install" :key="i" :style="{ color }">{{ text }}</span></span></code></pre>
    </div>
  </div>
</template>

<style scoped>
.install div[class*='language-'] {
  margin: 0;
  box-shadow: none;
  text-align: left;
}
</style>
