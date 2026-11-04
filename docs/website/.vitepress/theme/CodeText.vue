<script setup>
// Renders copy whose backticked parts are inline code.
defineProps({ text: { type: String, required: true } });
</script>

<template><template v-for="(part, i) in text.split('`')" :key="i"><code v-if="i % 2" class="inline-code">{{ part }}</code><template v-else>{{ part }}</template></template></template>
