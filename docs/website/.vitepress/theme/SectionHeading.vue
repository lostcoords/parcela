<script setup>
// A section title that links to itself, so any section can be shared as /#id.
defineProps({ id: { type: String, required: true } });
</script>

<template>
  <h2 :id="id" class="section-heading">
    <a class="heading-link" :href="`#${id}`"><slot /></a>
  </h2>
</template>

<style scoped>
h2 {
  scroll-margin-top: 96px;
}
.heading-link {
  position: relative;
  color: inherit;
  text-decoration: none;
}
/* The mark sits in the gutter before the title, so the title itself never moves. */
.heading-link::before {
  content: '#';
  position: absolute;
  right: 100%;
  padding-right: 0.3em;
  color: var(--subtle-foreground);
  font-weight: 500;
  opacity: 0;
  transition: opacity 0.15s ease;
}
.heading-link:hover::before,
.heading-link:focus-visible::before {
  opacity: 1;
}
.heading-link:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 4px;
  border-radius: 6px;
}
</style>
