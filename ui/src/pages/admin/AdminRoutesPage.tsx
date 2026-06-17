import { useEffect, useMemo, useState } from 'react';
import { DataCard, EmptyState } from '@/components/DataRow';
import {
  useCreateModelRouteMutation,
  useDeleteModelRouteMutation,
  useListModelRoutesQuery,
  useListProviderConnectionsQuery,
} from '@/features/api/mizanApi';

export function AdminRoutesPage() {
  const { data: routesData, isLoading: routeLoading } = useListModelRoutesQuery();
  const { data: providerData, isLoading: providerLoading } = useListProviderConnectionsQuery();

  const [providerConnectionId, setProviderConnectionId] = useState('');
  const [publicModel, setPublicModel] = useState('mizan/smart');
  const [upstreamModel, setUpstreamModel] = useState('gpt-4o-mini');

  const [createRoute, { isLoading: creating }] = useCreateModelRouteMutation();
  const [deleteRoute] = useDeleteModelRouteMutation();

  const providerOptions = useMemo(
    () => providerData?.data ?? [],
    [providerData],
  );

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
            onClick={() =>
              createRoute({
                provider_connection_id: providerConnectionId,
                public_model: publicModel.trim(),
                upstream_model: upstreamModel.trim(),
                pricing_input_per_1m_tokens: 250,
                pricing_output_per_1m_tokens: 1000,
              })
            }
            disabled={creating || providerLoading}
            className="rounded-lg border border-shell-border px-3 py-2"
          >
            Simpan
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
      </DataCard>

      <DataCard title="Model routes">
        {routeLoading ? (
          <p>Loading...</p>
        ) : routesData?.data.length === 0 ? (
          <EmptyState>Belum ada route.</EmptyState>
        ) : (
          <ul className="space-y-2">
            {routesData?.data.map((route) => (
              <li key={route.id} className="flex items-center justify-between rounded-lg border border-shell-border p-3">
                <div>
                  <p className="font-medium">{route.public_model} -> {route.upstream_model}</p>
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
        )}
      </DataCard>
    </div>
  );
}
