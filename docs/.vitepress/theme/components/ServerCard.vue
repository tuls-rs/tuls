<script setup lang="ts">
withDefaults(
  defineProps<{
    name: string;
    tagline: string;
    command: string;
    capabilities: string[];
    link: string;
    accent?: string;
  }>(),
  { accent: "#ea580c" },
);
</script>

<template>
  <a class="server-card" :href="link" :style="{ '--accent': accent }">
    <div class="server-card-top">
      <span class="server-card-name">{{ name }}</span>
      <span class="server-card-arrow" aria-hidden="true">→</span>
    </div>
    <p class="server-card-tagline">{{ tagline }}</p>
    <code class="server-card-command">{{ command }}</code>
    <div class="server-card-caps">
      <span v-for="cap in capabilities" :key="cap" class="cap-chip">{{
        cap
      }}</span>
    </div>
  </a>
</template>

<style scoped>
.server-card {
  --accent: #ea580c;
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
  padding: 1.4rem 1.5rem;
  border-radius: 14px;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-soft);
  transition:
    border-color 0.25s,
    transform 0.25s,
    box-shadow 0.25s,
    background-color 0.25s;
  text-decoration: none !important;
  margin: 10px auto;
}

.server-card:hover {
  border-color: var(--accent);
  transform: translateY(-3px);
  box-shadow: 0 10px 28px -12px rgba(234, 88, 12, 0.35);
  background: var(--vp-c-bg-soft-up);
}

.server-card-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.server-card-name {
  font-family: var(--vp-font-family-mono);
  font-size: 1.05rem;
  font-weight: 700;
  color: var(--accent);
}

.server-card-arrow {
  color: var(--vp-c-text-2);
  font-size: 1.1rem;
  transition: transform 0.25s;
}

.server-card:hover .server-card-arrow {
  transform: translateX(4px);
  color: var(--accent);
}

.server-card-tagline {
  margin: 0;
  color: var(--vp-c-text-1);
  font-size: 0.92rem;
  line-height: 1.55;
  flex: 1;
}

.server-card-command {
  display: block;
  padding: 0.45rem 0.7rem;
  border-radius: 8px;
  background: var(--vp-code-bg);
  color: var(--vp-code-color);
  font-size: 0.76rem;
  line-height: 1.5;
  overflow-x: auto;
  white-space: nowrap;
  max-width: 100%;
}

.server-card-caps {
  display: flex;
  flex-wrap: wrap;
  gap: 0.35rem;
}

.cap-chip {
  font-family: var(--vp-font-family-mono);
  font-size: 0.68rem;
  padding: 0.18rem 0.55rem;
  border-radius: 999px;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg);
  color: var(--vp-c-text-2);
}
</style>
