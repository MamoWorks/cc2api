import { ref } from 'vue';

export interface Toast {
  id: number;
  message: string;
  type: 'success' | 'error';
}

let nextId = 0;
const toasts = ref<Toast[]>([]);

/**
 * 添加一条 toast 消息
 * @param message 消息内容
 * @param type 类型
 * @param duration 持续时间（ms）
 */
function show(message: string, type: 'success' | 'error' = 'error', duration = 4000) {
  const id = nextId++;
  toasts.value.push({ id, message, type });
  setTimeout(() => {
    toasts.value = toasts.value.filter(t => t.id !== id);
  }, duration);
}

function dismiss(id: number) {
  toasts.value = toasts.value.filter(t => t.id !== id);
}

export function useToast() {
  return { toasts, show, dismiss };
}
