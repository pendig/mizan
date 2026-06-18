import { useEffect, useMemo, useState } from 'react';
import { DataCard, ErrorText, QueryState } from '@/components/DataRow';
import {
  useCreateModelRouteMutation,
  useDeleteModelRouteMutation,
  useListModelRoutesQuery,
  useListProviderConnectionsQuery,
} from '@/features/api/mizanApi';
import { extractApiErrorMessage } from '@/utils/errorHandling';

export function AdminRoutesPage() {
  const { data: routesData, isLoading: routeLoading, isError: routeError, refetch: refetchRoutes } =
    useListModelRoutesQuery();
  const {
    data: providerData,
    isLoading: providerLoading,
    isError: providerError,
    refetch: refetchProviders,
  } = useListProviderConnectionsQuery();

  const [providerConnectionId, setProviderConnectionId] = useState('');
  const [publicModel, setPublicModel] = useState('mizan/smart');
  const [upstreamModel, setUpstreamModel] = useState('gpt-4o-mini');

  const [createRoute, { isLoading: creating, error: createError }] = useCreateModelRouteMutation();
  const [deleteRoute] = useDeleteModelRouteMutation();

  const providerOptions = useMemo(() => providerData?.data ?? [], [providerData]);

  useEffect(() => {
    if (!providerConnectionId && providerOptions.length > 0) {
      setProviderConnectionId(providerOptions[0].id);
    }
  }, [providerConnectionId, providerOptions]);

  return (
    <div className="grid gap-4">
      <DataCard
        title="Tambah model route"
        action={
          <button
            type="button"
            onClick={() => {
              if (!providerConnectionId || creating || providerLoading) {
                return;
              }
              createRoute({
                provider_connection_id: providerConnectionId,
                public_model: publicModel.trim(),
                upstream_model: upstreamModel.trim(),
                pricing_input_per_1m_tokens: 250,
                pricing_output_per_1m_tokens: 1000,
              });
            }}
            disabled={creating || providerLoading || !providerConnectionId}
            className="rounded-lg border border-shell-border px-3 py-2"
          >
            {creating ? 'Menyimpan...' : 'Simpan'}
          </button>
        }
      >
        <div className="grid gap-2 sm:grid-cols-3">
          <select
            value={providerConnectionId}
            onChange={(event) => setProviderConnectionId(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
          >
            {providerOptions.length === 0 ? <option value="">Pilih provider dulu</option> : null}
            {providerOptions.map((provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.name} ({provider.id})
              </option>
            ))}
          </select>
          <input
            value={publicModel}
            onChange={(event) => setPublicModel(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Public model"
          />
          <input
            value={upstreamModel}
            onChange={(event) => setUpstreamModel(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Upstream model"
          />
        </div>
        {createError ? <ErrorText>{extractApiErrorMessage(createError, 'Gagal membuat route.')}</ErrorText> : null}
      </DataCard>

      <DataCard title="Model routes">
        <div className="mb-3 flex items-center gap-2">
          <button
            type="button"
            onClick={() => {
              void refetchRoutes();
              void refetchProviders();
            }}
            className="rounded-lg border border-shell-border px-2 py-1 text-xs"
          >
            Refresh
          </button>
          {routeError || providerError ? <p className="text-xs text-slate-400">Gagal load daftar</p> : null}
        </div>

        <QueryState
          isLoading={routeLoading}
          isError={routeError || providerError}
          isEmpty={!routeLoading && !routeError && !providerError && routesData?.data.length === 0}
          isEmptyText="Belum ada route."
        />

        {!routeLoading && !routeError && !providerError && routesData?.data && routesData.data.length > 0 ? (
          <ul className="space-y-2">
            {routesData.data.map((route) => (
              <li key={route.id} className="flex items-center justify-between rounded-lg border border-shell-border p-3">
                <div>
                  <p className="font-medium">
                    {route.public_model} {'\u2192'} {route.upstream_model}
                  </p>
                  <p className="text-xs text-slate-400">provider: {route.provider_connection_id}</p>
                </div>
                <button
                  type="button"
                  onClick={() => deleteRoute(route.id)}
                  className="rounded-lg border border-rose-500/60 px-2 py-1 text-sm text-rose-300"
                >
                  Hapus
                </button>
              </li>
            ))}
          </ul>
        ) : null}
      </DataCard>
    </div>
  );
}
