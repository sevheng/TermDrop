<template>
  <div ref="container" class="overflow-y-auto relative w-full h-full" @scroll="onScroll">
    <div :style="spacerStyle">
      <div v-for="(item, i) in visibleItems" :key="keyFn(item, startIndex + i)">
        <slot :item="item" :index="startIndex + i" />
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch, onMounted, nextTick } from 'vue'

const props = defineProps({
  items: { type: Array, required: true },
  itemHeight: { type: Number, default: 28 },
  keyFn: { type: Function, default: (item, index) => index },
  buffer: { type: Number, default: 5 },
})

const container = ref(null)
const scrollTop = ref(0)
const containerHeight = ref(0)

function measure() {
  if (container.value) {
    containerHeight.value = container.value.clientHeight
  }
}

onMounted(() => {
  nextTick(measure)
  window.addEventListener('resize', measure)
})

watch(() => props.items, () => nextTick(measure))

const totalHeight = computed(() => props.items.length * props.itemHeight)

const startIndex = computed(() => {
  const idx = Math.floor(scrollTop.value / props.itemHeight) - props.buffer
  return Math.max(0, idx)
})

const endIndex = computed(() => {
  const visibleCount = Math.ceil(containerHeight.value / props.itemHeight)
  const idx = startIndex.value + visibleCount + props.buffer * 2
  return Math.min(props.items.length, idx)
})

const visibleItems = computed(() =>
  props.items.slice(startIndex.value, endIndex.value)
)

const topPadding = computed(() => startIndex.value * props.itemHeight)
const bottomPadding = computed(() =>
  Math.max(0, totalHeight.value - topPadding.value - visibleItems.value.length * props.itemHeight)
)

const spacerStyle = computed(() => ({
  paddingTop: topPadding.value + 'px',
  paddingBottom: bottomPadding.value + 'px',
}))

function onScroll() {
  if (container.value) {
    scrollTop.value = container.value.scrollTop
  }
}
</script>
