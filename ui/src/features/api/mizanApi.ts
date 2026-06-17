import { createApi, fetchBaseQuery } from '@reduxjs/toolkit/query/react';
import { setSession } from '@/features/auth/authSlice';
import type { RootState } from '@/app/store';
import type {
  LoginPayload,
  LoginResponse,
  SessionResponse,
  ApiKeysResponse,
  ApiKeyCreateRequest,
  ApiKeyCreateResponse,
  ProviderConnectionsResponse,
  ProviderConnectionCreatePayload,
  ModelRoutesResponse,
  ModelRouteCreatePayload,
  WalletResponse,
  UsageListResponse,
  CreditGrantPayload,
  AdminUsageQueryFilters,
  DaemonNodesResponse,
  DaemonNodeCreatePayload,
  DaemonNodeCreateResponse,
  DaemonNodeRevokeResponse,
} from '@/app/types';

const DEFAULT_API_URL = 'http://127.0.0.1:18180';

const baseQuery = fetchBaseQuery({
  baseUrl: (import.meta.env.VITE_MIZAN_API_URL || DEFAULT_API_URL).replace(/\/$/, ''),
  prepareHeaders: (headers, { getState }) => {
    const state = getState() as RootState;
    const token = state.auth.token;
    if (token) {
      headers.set('Authorization', `Bearer ${token}`);
    }
    return headers;
  },
});

export const mizanApi = createApi({
  reducerPath: 'mizanApi',
  baseQuery,
  tagTypes: [
    'Auth',
    'ApiKeys',
    'ProviderConnections',
    'ModelRoutes',
    'Wallet',
    'Usage',
    'DaemonNodes',
  ],
  endpoints: (builder) => ({
    login: builder.mutation<LoginResponse, LoginPayload>({
      query: (body) => ({ url: '/auth/login', method: 'POST', body }),
      async onQueryStarted(_arg, { queryFulfilled, dispatch }) {
        try {
          const result = await queryFulfilled;
          dispatch(
            setSession({
              token: result.data.access_token,
              role: result.data.role,
              userId: result.data.user_id,
              expiresAt: result.data.expires_at,
            }),
          );
        } catch (_error) {
          // handled by mutation state in the UI
        }
      },
      invalidatesTags: ['Auth'],
    }),

    register: builder.mutation<{ user_id: string; email: string; role: string }, LoginPayload>({
      query: (body) => ({ url: '/auth/register', method: 'POST', body }),
    }),

    logout: builder.mutation<{ revoked: boolean }, void>({
      query: () => ({ url: '/auth/logout', method: 'POST' }),
      invalidatesTags: ['Auth', 'ApiKeys', 'Wallet', 'Usage', 'ProviderConnections', 'ModelRoutes'],
    }),

    me: builder.query<SessionResponse, void>({
      query: () => '/me',
      providesTags: ['Auth'],
    }),

    listApiKeys: builder.query<ApiKeysResponse, void>({
      query: () => '/api-keys',
      providesTags: ['ApiKeys'],
    }),

    createApiKey: builder.mutation<ApiKeyCreateResponse, ApiKeyCreateRequest>({
      query: (body) => ({ url: '/api-keys', method: 'POST', body }),
      invalidatesTags: ['ApiKeys'],
    }),

    revokeApiKey: builder.mutation<{ revoked: boolean; api_key_id: string }, string>({
      query: (id) => ({ url: `/api-keys/${id}`, method: 'DELETE' }),
      invalidatesTags: ['ApiKeys'],
    }),

    listProviderConnections: builder.query<ProviderConnectionsResponse, void>({
      query: () => '/admin/provider-connections',
      providesTags: ['ProviderConnections'],
    }),

    createProviderConnection: builder.mutation<
      { id: string; name: string; provider_type: string; auth_mode: string; base_url: string; enabled: boolean },
      ProviderConnectionCreatePayload
    >({
      query: (body) => ({
        url: '/admin/provider-connections',
        method: 'POST',
        body,
      }),
      invalidatesTags: ['ProviderConnections'],
    }),

    deleteProviderConnection: builder.mutation<{ removed: boolean; id: string }, string>({
      query: (id) => ({ url: `/admin/provider-connections/${id}`, method: 'DELETE' }),
      invalidatesTags: ['ProviderConnections'],
    }),

    listModelRoutes: builder.query<ModelRoutesResponse, void>({
      query: () => '/admin/model-routes',
      providesTags: ['ModelRoutes'],
    }),

    createModelRoute: builder.mutation<any, ModelRouteCreatePayload>({
      query: (body) => ({ url: '/admin/model-routes', method: 'POST', body }),
      invalidatesTags: ['ModelRoutes'],
    }),

    deleteModelRoute: builder.mutation<{ removed: boolean; id: string }, string>({
      query: (id) => ({ url: `/admin/model-routes/${id}`, method: 'DELETE' }),
      invalidatesTags: ['ModelRoutes'],
    }),

    listUsage: builder.query<UsageListResponse, { limit?: number; offset?: number }>({
      query: ({ limit, offset }) => {
        const params = new URLSearchParams();
        if (typeof limit === 'number') params.set('limit', String(limit));
        if (typeof offset === 'number') params.set('offset', String(offset));
        const suffix = params.toString() ? `?${params.toString()}` : '';
        return { url: `/v1/usage${suffix}` };
      },
      providesTags: ['Usage'],
    }),

    getWallet: builder.query<WalletResponse, void>({
      query: () => '/v1/credits',
      providesTags: ['Wallet'],
    }),

    listAdminUsage: builder.query<UsageListResponse, AdminUsageQueryFilters>({
      query: ({ userId, daemonNodeId, hostUserId, createdAfter, createdBefore, limit, offset }) => {
        const params = new URLSearchParams();
        if (userId) params.set('user_id', userId);
        if (daemonNodeId) params.set('daemon_node_id', daemonNodeId);
        if (hostUserId) params.set('host_user_id', hostUserId);
        if (createdAfter) params.set('created_after', createdAfter);
        if (createdBefore) params.set('created_before', createdBefore);
        if (typeof limit === 'number') params.set('limit', String(limit));
        if (typeof offset === 'number') params.set('offset', String(offset));
        const suffix = params.toString() ? `?${params.toString()}` : '';
        return { url: `/admin/usage${suffix}` };
      },
      providesTags: ['Usage'],
    }),

    grantCredits: builder.mutation<{ user_id: string; balance_microcredits: number }, CreditGrantPayload>({
      query: ({ user_id, amount_microcredits, reason }) => ({
        url: `/admin/users/${user_id}/credits/grant`,
        method: 'POST',
        body: { amount_microcredits, reason },
      }),
      invalidatesTags: ['Wallet', 'Usage'],
    }),

    listDaemonNodes: builder.query<DaemonNodesResponse, void>({
      query: () => '/admin/daemon-nodes',
      providesTags: ['DaemonNodes'],
    }),

    createDaemonNode: builder.mutation<DaemonNodeCreateResponse, DaemonNodeCreatePayload>({
      query: (body) => ({
        url: '/admin/daemon-nodes',
        method: 'POST',
        body,
      }),
      invalidatesTags: ['DaemonNodes'],
    }),

    revokeDaemonNode: builder.mutation<DaemonNodeRevokeResponse, string>({
      query: (id) => ({
        url: `/admin/daemon-nodes/${id}`,
        method: 'DELETE',
      }),
      invalidatesTags: ['DaemonNodes'],
    }),
  }),
});

export const {
  useLoginMutation,
  useRegisterMutation,
  useLogoutMutation,
  useMeQuery,
  useListApiKeysQuery,
  useCreateApiKeyMutation,
  useRevokeApiKeyMutation,
  useListProviderConnectionsQuery,
  useCreateProviderConnectionMutation,
  useDeleteProviderConnectionMutation,
  useListModelRoutesQuery,
  useCreateModelRouteMutation,
  useDeleteModelRouteMutation,
  useListUsageQuery,
  useGetWalletQuery,
  useListAdminUsageQuery,
  useGrantCreditsMutation,
  useListDaemonNodesQuery,
  useCreateDaemonNodeMutation,
  useRevokeDaemonNodeMutation,
} = mizanApi;
