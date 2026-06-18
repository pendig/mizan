import { createSlice, type PayloadAction } from '@reduxjs/toolkit';
import type { UserRole } from '@/app/types';
import type { RootState } from '@/app/store';

interface AuthState {
  token: string | null;
  role: UserRole | null;
  userId: string | null;
  expiresAt: string | null;
}

const STORAGE_KEY = 'mizan_access_token';
const ROLE_STORAGE_KEY = 'mizan_user_role';
const USER_ID_STORAGE_KEY = 'mizan_user_id';
const EXPIRES_AT_STORAGE_KEY = 'mizan_token_expires_at';

const clearStoredSession = () => {
  localStorage.removeItem(STORAGE_KEY);
  localStorage.removeItem(ROLE_STORAGE_KEY);
  localStorage.removeItem(USER_ID_STORAGE_KEY);
  localStorage.removeItem(EXPIRES_AT_STORAGE_KEY);
};

const parseInitial = (): AuthState => {
  const token = localStorage.getItem(STORAGE_KEY);
  const role = localStorage.getItem(ROLE_STORAGE_KEY);
  const userId = localStorage.getItem(USER_ID_STORAGE_KEY);
  const expiresAt = localStorage.getItem(EXPIRES_AT_STORAGE_KEY);

  if (!token) {
    return { token: null, role: null, userId: null, expiresAt: null };
  }

  if (expiresAt && Date.parse(expiresAt) <= Date.now()) {
    clearStoredSession();
    return { token: null, role: null, userId: null, expiresAt: null };
  }

  return {
    token,
    role: role ?? null,
    userId: userId ?? null,
    expiresAt,
  };
};

const initialState: AuthState = parseInitial();

const authSlice = createSlice({
  name: 'auth',
  initialState,
  reducers: {
    setSession: (state, action: PayloadAction<AuthState>) => {
      state.token = action.payload.token;
      state.role = action.payload.role;
      state.userId = action.payload.userId;
      state.expiresAt = action.payload.expiresAt;

      if (action.payload.token) {
        localStorage.setItem(STORAGE_KEY, action.payload.token);
        if (action.payload.role) {
          localStorage.setItem(ROLE_STORAGE_KEY, action.payload.role);
        }
        if (action.payload.userId) {
          localStorage.setItem(USER_ID_STORAGE_KEY, action.payload.userId);
        }
        if (action.payload.expiresAt) {
          localStorage.setItem(EXPIRES_AT_STORAGE_KEY, action.payload.expiresAt);
        }
      }
    },
    clearSession: (state) => {
      state.token = null;
      state.role = null;
      state.userId = null;
      state.expiresAt = null;
      clearStoredSession();
    },
  },
});

export const { setSession, clearSession } = authSlice.actions;
export default authSlice.reducer;

export const selectAuth = (state: RootState) => state.auth;
export const selectToken = (state: RootState) => state.auth.token;
export const selectRole = (state: RootState) => state.auth.role;
export const selectUserId = (state: RootState) => state.auth.userId;
