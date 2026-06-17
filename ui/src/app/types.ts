export type UserRole = 'admin' | 'member' | string;

export interface LoginPayload {
  email: string;
  password: string;
}

export interface LoginResponse {
  access_token: string;
  token_type: string;
  expires_at: string;
  user_id: string;
  role: UserRole;
}

export interface SessionResponse {
  user_id: string;
  role: UserRole;
}

export interface ApiKey {
  id: string;
  label?: string;
  created_at: string;
}

export interface ApiKeyCreateRequest {
  label?: string;
}

export interface ApiKeyCreateResponse {
  id: string;
  key: string;
  label?: string;
  created_at: string;
}

export interface ApiKeysResponse {
  keys: ApiKey[];
}

export interface ProviderConnection {
  id: string;
  name: string;
  provider_type: string;
  auth_mode: string;
  base_url: string;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface ProviderConnectionCreatePayload {
  name: string;
  provider_type: string;
  base_url?: string;
  api_key_encrypted?: string;
  auth_mode?: string;
  enabled?: boolean;
}

export interface ProviderConnectionsResponse {
  data: ProviderConnection[];
}

export interface ModelRoute {
  id: string;
  provider_connection_id: string;
  public_model: string;
  upstream_model: string;
  max_tokens?: number;
  pricing_input_per_1m_tokens: number;
  pricing_output_per_1m_tokens: number;
  enabled: boolean;
  created_at: string;
  updated_at: string;
  provider_name?: string;
}

export interface ModelRouteCreatePayload {
  provider_connection_id: string;
  public_model: string;
  upstream_model: string;
  max_tokens?: number;
  pricing_input_per_1m_tokens?: number;
  pricing_output_per_1m_tokens?: number;
  enabled?: boolean;
}

export interface ModelRoutesResponse {
  data: ModelRoute[];
}

export interface WalletResponse {
  user_id: string;
  balance_microcredits: number;
}

export interface UsageEvent {
  id: string;
  request_id: string;
  user_id?: string;
  api_key_id?: string;
  provider_id?: string;
  route_id?: string;
  daemon_node_id?: string;
  model: string;
  usage_prompt_tokens: number;
  usage_completion_tokens: number;
  usage_total_tokens: number;
  usage_estimated: boolean;
  status_code: number;
  latency_ms: number;
  created_at: string;
}

export interface UsageListResponse {
  data: UsageEvent[];
}

export interface CreditGrantPayload {
  user_id: string;
  amount_microcredits: number;
  reason?: string;
}

export interface AdminUsageQueryFilters {
  userId?: string;
  daemonNodeId?: string;
  hostUserId?: string;
  createdAfter?: string;
  createdBefore?: string;
  limit?: number;
  offset?: number;
}

export interface DaemonNode {
  id: string;
  host_user_id?: string;
  label?: string;
  hostname?: string;
  public_key?: string;
  status: string;
  revoked: boolean;
  disabled: boolean;
  last_seen_at?: string;
  capabilities: DaemonNodeCapabilities;
  created_at: string;
  updated_at: string;
}

export interface DaemonNodeCapabilities {
  provider_family?: string;
  model_ids: string[];
  max_concurrency?: number;
  region?: string;
  labels: string[];
  health_status?: string;
  metadata?: Record<string, unknown>;
  pricing_metadata?: Record<string, unknown>;
}

export interface DaemonNodesResponse {
  data: DaemonNode[];
}

export interface DaemonNodeCreatePayload {
  host_user_id?: string;
  label?: string;
  hostname?: string;
  public_key?: string;
}

export interface DaemonNodeCreateResponse {
  id: string;
  token: string;
  token_type: string;
  status: string;
  host_user_id?: string;
  label?: string;
  hostname?: string;
  public_key?: string;
  created_at: string;
}

export interface DaemonNodeRevokeResponse {
  id: string;
  revoked: boolean;
}
