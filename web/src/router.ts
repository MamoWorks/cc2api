import { createRouter, createWebHistory } from 'vue-router';
import { setAuth, api } from './api';

/** 认证状态 */
let authenticated = false;

/**
 * 验证已保存的凭证
 * @returns 验证是否通过
 */
async function tryRestoreAuth(): Promise<boolean> {
  const saved = localStorage.getItem('cc2api_auth');
  if (!saved) return false;
  setAuth(saved);
  try {
    await api.getDashboard();
    authenticated = true;
    return true;
  } catch {
    localStorage.removeItem('cc2api_auth');
    setAuth('');
    return false;
  }
}

/** 初始化 Promise，确保只验证一次 */
let initPromise: Promise<boolean> | null = null;

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/login',
      name: 'login',
      component: () => import('./components/Login.vue'),
    },
    {
      path: '/',
      name: 'dashboard',
      component: () => import('./components/Dashboard.vue'),
      meta: { requiresAuth: true },
      children: [
        { path: '', name: 'accounts', component: () => import('./components/Accounts.vue') },
        { path: 'tokens', name: 'tokens', component: () => import('./components/Tokens.vue') },
      ],
    },
  ],
});

router.beforeEach(async (to) => {
  if (!authenticated && !initPromise) {
    initPromise = tryRestoreAuth();
  }
  if (initPromise) {
    await initPromise;
    initPromise = null;
  }

  if (to.meta.requiresAuth && !authenticated) {
    return { name: 'login' };
  }
  if (to.name === 'login' && authenticated) {
    return { name: 'dashboard' };
  }
});

/**
 * 登录并跳转到仪表盘
 * @param password 管理员密码
 */
export async function login(password: string): Promise<void> {
  setAuth(password);
  await api.getDashboard();
  authenticated = true;
  localStorage.setItem('cc2api_auth', password);
  await router.push({ name: 'dashboard' });
}

/** 退出登录并跳转到登录页 */
export function logout(): void {
  authenticated = false;
  localStorage.removeItem('cc2api_auth');
  setAuth('');
  router.push({ name: 'login' });
}

export default router;
