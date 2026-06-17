import { useState } from 'react';
import { useMemo } from 'react';
import { DataCard, QueryState, ErrorText } from '@/components/DataRow';
import {
  useCreateProviderConnectionMutation,
  useDeleteProviderConnectionMutation,
  useListProviderConnectionsQuery,
  useListModelRoutesQuery,
} from '@/features/api/mizanApi';

export function AdminProvidersPage() {
  const { data, isLoading, isError, refetch } = useListProviderConnectionsQuery();
  const [createProvider, { isLoading: creating, error: createError }] = useCreateProviderConnectionMutation();
  const { data: routeData } = useListModelRoutesQuery();
  const [deleteProvider] = useDeleteProviderConnectionMutation();

  const [name, setName] = useState('OpenAI Prod');
  const [providerType, setProviderType] = useState('openai');
  const [baseUrl, setBaseUrl] = useState('https://api.openai.com/v1');
  const providers = data?.data ?? [];
  const routeByProvider = useMemo(() => {
    const counts = new Map<string, number>();
    for (const route of routeData?.data ?? []) {
      const current = counts.get(route.provider_connection_id) ?? 0;
      counts.set(route.provider_connection_id, current + 1);
    }
    return counts;
  }, [routeData]);

  const hasRouteCountText = (count: number) => {
    if (count > 1) return `${count} routes`;
    if (count === 1) return '1 route';
    return 'No route';
  };

  const canSubmit = name.trim().length > 0 && providerType.trim().length > 0;

  return (
    <div className="grid gap-4">
      <DataCard
        title="Tambah provider"
        action={
          <button
            type="button"
            onClick={() => {
              if (!canSubmit || creating) {
                return;
              }
              void createProvider({
                name: name.trim(),
                provider_type: providerType.trim(),
                base_url: baseUrl.trim(),
                enabled: true,
              });
            }}
            disabled={creating || !canSubmit}
            className="rounded-lg border border-shell-border px-3 py-2"
          >
            {creating ? 'Menyimpan...' : 'Simpan'}
          </button>
        }
      >
        <div className="grid gap-2 sm:grid-cols-2">
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Nama koneksi"
          />
          <input
            value={providerType}
            onChange={(event) => setProviderType(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Tipe provider"
          />
          <input
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2 sm:col-span-2"
            placeholder="Base URL"
          />
        </div>
      </DataCard>

      <DataCard title="Provider connections">
        <div className="mb-3">
          <button
            type="button"
            onClick={() => refetch()}
            className="rounded-lg border border-shell-border px-2 py-1 text-xs"
          >
            Refresh
          </button>
        </div>

        <QueryState
          isLoading={isLoading}
          isError={isError}
          isEmpty={!isLoading && !isError && providers.length === 0}
          isEmptyText="Belum ada provider."
        />
        {createError ? <ErrorText>{extractErrorMessage(createError)}</ErrorText> : null}

        {!isLoading && !isError && providers.length > 0 ? (
          <ul className="space-y-2">
            {providers.map((provider) => {
              const routes = routeByProvider.get(provider.id) ?? 0;
              const providerHealth = provider.enabled ? (routes > 0 ? 'healthy' : 'idle') : 'disabled';
              return (
                <li key={provider.id} className="rounded-lg border border-shell-border p-3">
                  <div className="flex items-center justify-between gap-2">
                    <p className="font-medium">{provider.name}</p>
                    <div className="flex flex-wrap justify-end gap-2">
                      <span className="rounded-lg border border-shell-border px-2 py-1 text-xs text-slate-200">
                        {providerTypeLabel(provider.provider_type)}
                      </span>
                      <span
                        className={`rounded-lg border px-2 py-1 text-xs ${
                          providerHealth === 'healthy'
                            ? 'border-emerald-500/40 text-emerald-200'
                            : providerHealth === 'disabled'
                              ? 'border-rose-500/40 text-rose-200'
                              : 'border-slate-500/40 text-slate-300'
                        }`}
                      >
                        {providerHealth}
                      </span>
                      <span className="rounded-lg border border-shell-border px-2 py-1 text-xs text-slate-300">
                        {hasRouteCountText(routes)}
                      </span>
                    </div>
                  </div>
                  <p className="text-xs text-slate-400">
                    {provider.base_url} · routes: {routes}
                  </p>
                  <div className="mt-2">
                    <button
                      onClick={() => deleteProvider(provider.id)}
                      type="button"
                      className="rounded-lg border border-rose-500/60 px-2 py-1 text-sm text-rose-300"
                    >
                      Hapus
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        ) : null}
      </DataCard>
    </div>
  );
}

function providerTypeLabel(input: string) {
  return input.trim().length > 0 ? input.trim() : 'unknown';
}

function extractErrorMessage(error: unknown) {
  if (typeof error === 'string') {
    return error;
  }
  if (error && typeof error === 'object' && 'data' in error) {
    const maybeData = (error as { data?: unknown }).data;
    if (maybeData && typeof maybeData === 'object') {
      const message =
        (maybeData as { error?: string; message?: string }).error ??
        (maybeData as { message?: string }).message;
      if (typeof message === 'string') {
        return message;
      }
    }
  }
  return 'Gagal menyimpan provider.';
}
