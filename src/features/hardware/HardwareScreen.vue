<script setup lang="ts">
// The Hardware screen (spec 7.7): the grid of devices, and the Disk screen, which opens from the
// Disk tile. Which one shows is the route's sub-view ('devices' or 'disk'), so a refresh in the
// browser and the "Go to Disk" command both land in the right place.
import { computed, defineAsyncComponent } from 'vue'
import { useNav } from '@/nav/useNav'
import HardwareGrid from './HardwareGrid.vue'

// The Disk screen (and the treemap code) loads only when it is opened.
const DiskScreen = defineAsyncComponent(() => import('../disk/DiskScreen.vue'))

const nav = useNav()
const showsDisk = computed(() => nav.route.value.sub === 'disk')
</script>

<template>
  <!-- One component at a time, each with its own keyboard controller. -->
  <DiskScreen v-if="showsDisk" />
  <HardwareGrid v-else />
</template>
