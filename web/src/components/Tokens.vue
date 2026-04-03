<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { api, type ApiToken, type Account } from '../api';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Badge } from '@/components/ui/badge';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog';
import { useToast } from '../composables/useToast';

const { show: toast } = useToast();

/** 令牌列表 */
const tokens = ref<ApiToken[]>([]);
/** 所有账号（用于选择可用/不可用账号） */
const allAccounts = ref<Account[]>([]);
/** 表单弹窗是否可见 */
const showForm = ref(false);
/** 删除确认弹窗 */
const showDeleteConfirm = ref(false);
/** 待删除令牌 ID */
const deleteTargetId = ref<number | null>(null);
/** 当前编辑的令牌 */
const editing = ref<ApiToken | null>(null);
/** 表单数据 */
const form = ref({ name: '', allowed_accounts: '', blocked_accounts: '' });
/** 已复制的令牌 ID */
const copiedId = ref<number | null>(null);

/** 加载令牌列表 */
async function load() {
  try {
    const res = await api.listTokens(1, 100);
    tokens.value = res.data ?? [];
  } catch {
    tokens.value = [];
  }
}

/** 加载所有账号 */
async function loadAccounts() {
  try {
    const res = await api.listAccounts(1, 100);
    allAccounts.value = res.data ?? [];
  } catch {
    allAccounts.value = [];
  }
}

onMounted(() => {
  load();
  loadAccounts();
});

/** 打开新建令牌弹窗 */
function openCreate() {
  editing.value = null;
  form.value = { name: '', allowed_accounts: '', blocked_accounts: '' };
  showForm.value = true;
}

/**
 * 打开编辑令牌弹窗
 * @param t 要编辑的令牌
 */
function openEdit(t: ApiToken) {
  editing.value = t;
  form.value = {
    name: t.name,
    allowed_accounts: t.allowed_accounts,
    blocked_accounts: t.blocked_accounts,
  };
  showForm.value = true;
}

/** 保存令牌 */
async function save() {
  try {
    if (editing.value) {
      await api.updateToken(editing.value.id, {
        name: form.value.name,
        allowed_accounts: form.value.allowed_accounts,
        blocked_accounts: form.value.blocked_accounts,
      });
    } else {
      await api.createToken({
        name: form.value.name,
        allowed_accounts: form.value.allowed_accounts,
        blocked_accounts: form.value.blocked_accounts,
      });
    }
    showForm.value = false;
    await load();
  } catch (e: unknown) {
    toast((e as Error).message || '保存失败');
  }
}

/** 确认删除 */
function confirmDelete(id: number) {
  deleteTargetId.value = id;
  showDeleteConfirm.value = true;
}

/** 执行删除 */
async function executeDelete() {
  if (deleteTargetId.value === null) return;
  try {
    await api.deleteToken(deleteTargetId.value);
    showDeleteConfirm.value = false;
    deleteTargetId.value = null;
    await load();
  } catch (e: unknown) {
    toast((e as Error).message || '删除失败');
  }
}

/** 切换令牌状态 */
async function toggleStatus(t: ApiToken) {
  try {
    const newStatus = t.status === 'active' ? 'disabled' : 'active';
    await api.updateToken(t.id, { status: newStatus });
    await load();
  } catch (e: unknown) {
    toast((e as Error).message || '操作失败');
  }
}

/**
 * 复制令牌到剪贴板
 * @param t 令牌对象
 */
async function copyToken(t: ApiToken) {
  await navigator.clipboard.writeText(t.token);
  copiedId.value = t.id;
  setTimeout(() => { copiedId.value = null; }, 2000);
}

/**
 * 遮蔽令牌显示
 * @param token 令牌值
 */
function maskToken(token: string): string {
  if (token.length <= 12) return token;
  return token.slice(0, 7) + '...' + token.slice(-4);
}

/**
 * 解析 ID 列表为账号名显示
 * @param ids 逗号分隔的 ID
 */
function formatAccountIds(ids: string): string {
  if (!ids) return '不限制';
  return ids.split(',').map(id => {
    const acc = allAccounts.value.find(a => a.id === Number(id.trim()));
    return acc ? (acc.name || acc.email) : `#${id.trim()}`;
  }).join(', ');
}

/**
 * 获取状态徽章样式
 * @param status 令牌状态
 */
function statusStyle(status: string): { class: string; label: string } {
  if (status === 'active') return { class: 'bg-emerald-50 text-emerald-700 border-emerald-200', label: '活跃' };
  return { class: 'bg-gray-100 text-gray-500 border-gray-200', label: '停用' };
}

/**
 * 切换账号 ID 在输入框中的选中状态
 * @param field 目标字段
 * @param id 账号 ID
 */
function toggleAccountId(field: 'allowed' | 'blocked', id: number) {
  const key = field === 'allowed' ? 'allowed_accounts' : 'blocked_accounts';
  const ids = form.value[key]
    .split(',')
    .map(s => s.trim())
    .filter(s => s !== '');

  const idx = ids.indexOf(String(id));
  if (idx >= 0) {
    ids.splice(idx, 1);
  } else {
    ids.push(String(id));
  }
  form.value[key] = ids.join(',');
}

/**
 * 判断账号 ID 是否已选中
 * @param field 目标字段
 * @param id 账号 ID
 */
function isAccountSelected(field: 'allowed' | 'blocked', id: number): boolean {
  const key = field === 'allowed' ? 'allowed_accounts' : 'blocked_accounts';
  return form.value[key]
    .split(',')
    .map(s => s.trim())
    .includes(String(id));
}
</script>

<template>
  <div class="space-y-4">
    <!-- 标题栏 -->
    <div class="flex justify-between items-center">
      <h2 class="text-lg font-semibold text-[#29261e]">令牌管理</h2>
      <Button
        @click="openCreate"
        class="bg-[#c4704f] hover:bg-[#b5623f] text-white font-medium rounded-xl transition-all duration-200 hover:shadow-md"
      >
        创建令牌
      </Button>
    </div>

    <!-- 令牌卡片列表 -->
    <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
      <Card
        v-for="t in tokens"
        :key="t.id"
        class="bg-white border-[#e8e2d9] rounded-xl hover:shadow-md transition-all duration-200 overflow-hidden"
      >
        <div class="p-5 space-y-3">
          <!-- 头部：名称 + 状态 -->
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-2 min-w-0">
              <div class="w-8 h-8 rounded-lg bg-[#c4704f]/10 flex items-center justify-center flex-shrink-0">
                <svg class="w-4 h-4 text-[#c4704f]" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 5.25a3 3 0 0 1 3 3m3 0a6 6 0 0 1-7.029 5.912c-.563-.097-1.159.026-1.563.43L10.5 17.25H8.25v2.25H6v2.25H2.25v-2.818c0-.597.237-1.17.659-1.591l6.499-6.499c.404-.404.527-1 .43-1.563A6 6 0 1 1 21.75 8.25Z" />
                </svg>
              </div>
              <div class="min-w-0">
                <p class="text-sm font-medium text-[#29261e] truncate">{{ t.name || '未命名令牌' }}</p>
                <p class="text-xs text-[#8c8475]">{{ new Date(t.created_at).toLocaleDateString('zh-CN') }}</p>
              </div>
            </div>
            <Badge :class="statusStyle(t.status).class" class="border text-xs font-medium flex-shrink-0">
              {{ statusStyle(t.status).label }}
            </Badge>
          </div>

          <!-- Token 值 -->
          <div class="pt-2 border-t border-[#f0ebe4]">
            <div class="flex items-center gap-2">
              <code class="font-mono text-[11px] text-[#8c8475] truncate flex-1">{{ maskToken(t.token) }}</code>
              <Button
                variant="ghost"
                size="sm"
                @click="copyToken(t)"
                class="h-7 px-2 text-[#8c8475] hover:text-[#29261e] hover:bg-[#f0ebe4] flex-shrink-0"
              >
                {{ copiedId === t.id ? '已复制' : '复制' }}
              </Button>
            </div>
          </div>

          <!-- 账号限制 -->
          <div class="space-y-2">
            <div>
              <p class="text-[10px] text-[#b5b0a6] uppercase tracking-wider mb-0.5">可用账号</p>
              <p class="text-xs text-[#8c8475] truncate">{{ formatAccountIds(t.allowed_accounts) }}</p>
            </div>
            <div>
              <p class="text-[10px] text-[#b5b0a6] uppercase tracking-wider mb-0.5">不可用账号</p>
              <p class="text-xs text-[#8c8475] truncate">{{ formatAccountIds(t.blocked_accounts) }}</p>
            </div>
          </div>

          <!-- 操作按钮 -->
          <div class="flex items-center gap-2 pt-2 border-t border-[#f0ebe4]">
            <Button
              variant="ghost"
              size="sm"
              @click="openEdit(t)"
              class="text-[#8c8475] hover:text-[#29261e] hover:bg-[#f0ebe4] h-8 px-3 text-xs flex-1"
            >
              编辑
            </Button>
            <Button
              variant="ghost"
              size="sm"
              @click="toggleStatus(t)"
              class="h-8 px-3 text-xs flex-1"
              :class="t.status === 'active'
                ? 'text-[#8c8475] hover:text-[#29261e] hover:bg-[#f0ebe4]'
                : 'text-emerald-600 hover:text-emerald-700 hover:bg-emerald-50'"
            >
              {{ t.status === 'active' ? '停用' : '启用' }}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              @click="confirmDelete(t.id)"
              class="text-red-400 hover:text-red-500 hover:bg-red-50 h-8 px-3 text-xs flex-1"
            >
              删除
            </Button>
          </div>
        </div>
      </Card>

      <!-- 空状态 -->
      <div
        v-if="tokens.length === 0"
        class="col-span-full flex flex-col items-center justify-center py-16 text-[#b5b0a6]"
      >
        <div class="w-12 h-12 rounded-xl bg-[#f0ebe4] flex items-center justify-center mb-3">
          <svg class="w-6 h-6 text-[#c4704f]/50" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 5.25a3 3 0 0 1 3 3m3 0a6 6 0 0 1-7.029 5.912c-.563-.097-1.159.026-1.563.43L10.5 17.25H8.25v2.25H6v2.25H2.25v-2.818c0-.597.237-1.17.659-1.591l6.499-6.499c.404-.404.527-1 .43-1.563A6 6 0 1 1 21.75 8.25Z" />
          </svg>
        </div>
        <p class="text-sm">暂无令牌，点击"创建令牌"开始</p>
      </div>
    </div>

    <!-- 新建/编辑令牌弹窗 -->
    <Dialog v-model:open="showForm">
      <DialogContent class="bg-white border-[#e8e2d9] rounded-2xl text-[#29261e] sm:max-w-md">
        <DialogHeader>
          <DialogTitle class="text-[#29261e] text-lg">{{ editing ? '编辑令牌' : '创建令牌' }}</DialogTitle>
          <DialogDescription class="text-[#8c8475]">
            {{ editing ? '修改令牌设置' : '创建新的 API 令牌，令牌将自动生成' }}
          </DialogDescription>
        </DialogHeader>

        <form @submit.prevent="save" class="space-y-4 mt-2">
          <div class="space-y-2">
            <Label class="text-[#5c5647] text-sm">备注名（选填）</Label>
            <Input
              v-model="form.name"
              placeholder="例如：生产环境、测试用"
              class="bg-[#f9f6f1] border-[#e8e2d9] text-[#29261e] placeholder-[#b5b0a6] focus:border-[#c4704f] focus:ring-[#c4704f]/20"
            />
          </div>
          <div class="space-y-2">
            <Label class="text-[#5c5647] text-sm">可用账号（选填，留空不限制）</Label>
            <Input
              v-model="form.allowed_accounts"
              placeholder="账号 ID，逗号分隔"
              class="bg-[#f9f6f1] border-[#e8e2d9] text-[#29261e] placeholder-[#b5b0a6] focus:border-[#c4704f] focus:ring-[#c4704f]/20"
            />
            <div v-if="allAccounts.length" class="flex flex-wrap gap-1.5">
              <button
                v-for="a in allAccounts"
                :key="a.id"
                type="button"
                @click="toggleAccountId('allowed', a.id)"
                class="text-[10px] px-2 py-0.5 rounded-md border transition-colors"
                :class="isAccountSelected('allowed', a.id)
                  ? 'bg-[#c4704f]/10 border-[#c4704f]/30 text-[#c4704f]'
                  : 'bg-[#f9f6f1] border-[#e8e2d9] text-[#8c8475] hover:border-[#c4704f]/30'"
              >
                #{{ a.id }} {{ a.name || a.email }}
              </button>
            </div>
          </div>
          <div class="space-y-2">
            <Label class="text-[#5c5647] text-sm">不可用账号（选填）</Label>
            <Input
              v-model="form.blocked_accounts"
              placeholder="账号 ID，逗号分隔"
              class="bg-[#f9f6f1] border-[#e8e2d9] text-[#29261e] placeholder-[#b5b0a6] focus:border-[#c4704f] focus:ring-[#c4704f]/20"
            />
            <div v-if="allAccounts.length" class="flex flex-wrap gap-1.5">
              <button
                v-for="a in allAccounts"
                :key="a.id"
                type="button"
                @click="toggleAccountId('blocked', a.id)"
                class="text-[10px] px-2 py-0.5 rounded-md border transition-colors"
                :class="isAccountSelected('blocked', a.id)
                  ? 'bg-red-50 border-red-200 text-red-500'
                  : 'bg-[#f9f6f1] border-[#e8e2d9] text-[#8c8475] hover:border-red-200'"
              >
                #{{ a.id }} {{ a.name || a.email }}
              </button>
            </div>
          </div>

          <DialogFooter class="gap-2 pt-2">
            <Button
              type="button"
              variant="ghost"
              @click="showForm = false"
              class="text-[#8c8475] hover:text-[#29261e] hover:bg-[#f0ebe4]"
            >
              取消
            </Button>
            <Button
              type="submit"
              class="bg-[#c4704f] hover:bg-[#b5623f] text-white font-medium rounded-xl transition-all duration-200"
            >
              保存
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>

    <!-- 删除确认弹窗 -->
    <Dialog v-model:open="showDeleteConfirm">
      <DialogContent class="bg-white border-[#e8e2d9] rounded-2xl text-[#29261e] sm:max-w-sm">
        <DialogHeader>
          <DialogTitle class="text-[#29261e]">确认删除</DialogTitle>
          <DialogDescription class="text-[#8c8475]">
            此操作不可撤销，删除后使用该令牌的客户端将无法访问。
          </DialogDescription>
        </DialogHeader>
        <DialogFooter class="gap-2 pt-4">
          <Button
            variant="ghost"
            @click="showDeleteConfirm = false"
            class="text-[#8c8475] hover:text-[#29261e] hover:bg-[#f0ebe4]"
          >
            取消
          </Button>
          <Button
            @click="executeDelete"
            class="bg-red-500 hover:bg-red-600 text-white font-medium rounded-xl transition-all duration-200"
          >
            删除
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </div>
</template>
