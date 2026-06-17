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

const parseInitial = (): AuthState => {
  const token = localStorage.getItem(STORAGE_KEY);
  const role = localStorage.getItem('mizan_user_role');
  const userId = localStorage.getItem('mizan_user_id');
  const expiresAt = localStorage.getItem('mizan_token_expires_at');

  if (!token) {
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
          localStorage.setItem('mizan_user_role', action.payload.role);
        }
        if (action.payload.userId) {
          localStorage.setItem('mizan_user_id', action.payload.userId);
        }
        if (action.payload.expiresAt) {
          localStorage.setItem('mizan_token_expires_at', action.payload.expiresAt);
        }
      }
    },
    clearSession: (state) => {
      state.token = null;
      state.role = null;
      state.userId = null;
      state.expiresAt = null;
      localStorage.removeItem(STORAGE_KEY);
      localStorage.removeItem('mizan_user_role');
      localStorage.removeItem('mizan_user_id');
      localStorage.removeItem('mizan_token_expires_at');
    },
  },
});

export const { setSession, clearSession } = authSlice.actions;
export default authSlice.reducer;

export const selectAuth = (state: RootState) => state.auth;
export const selectToken = (state: RootState) => state.auth.token;
export const selectRole = (state: RootState) => state.auth.role;
export const selectUserId = (state: RootState) => state.auth.userId;
