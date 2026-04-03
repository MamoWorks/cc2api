<script setup lang="ts">
import { useToast } from './composables/useToast';

const { toasts, dismiss } = useToast();
</script>

<template>
  <div class="min-h-screen bg-[#f9f6f1]">
    <router-view />

    <!-- Toast 容器 -->
    <div class="fixed top-4 right-4 z-50 space-y-2 max-w-sm">
      <transition-group name="toast">
        <div
          v-for="t in toasts"
          :key="t.id"
          @click="dismiss(t.id)"
          class="px-4 py-3 rounded-xl shadow-lg border cursor-pointer text-sm leading-relaxed"
          :class="t.type === 'error'
            ? 'bg-white border-red-200 text-red-600'
            : 'bg-white border-emerald-200 text-emerald-600'"
        >
          {{ t.message }}
        </div>
      </transition-group>
    </div>
  </div>
</template>

<style scoped>
.toast-enter-active { animation: toast-in 0.25s ease-out; }
.toast-leave-active { animation: toast-out 0.2s ease-in forwards; }
@keyframes toast-in { from { opacity: 0; transform: translateX(100%); } to { opacity: 1; transform: translateX(0); } }
@keyframes toast-out { from { opacity: 1; transform: translateX(0); } to { opacity: 0; transform: translateX(100%); } }
</style>
