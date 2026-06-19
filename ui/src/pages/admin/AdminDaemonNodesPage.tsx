import { useMemo, useState } from 'react';
import { DataCard, ErrorText, QueryState } from '@/components/DataRow';
import {
  useCreateDaemonNodeMutation,
  useListDaemonNodesQuery,
  useRevokeDaemonNodeMutation,
} from '@/features/api/mizanApi';
import { extractApiErrorMessage } from '@/utils/errorHandling';

export function AdminDaemonNodesPage() {
  const {
    data,
    isLoading,
    isError,
    refetch,
  } = useListDaemonNodesQuery();
  const [createNode, { isLoading: creating, error: createError }] = useCreateDaemonNodeMutation();
  const [revokeNode, { error: revokeError }] = useRevokeDaemonNodeMutation();

  const [label, setLabel] = useState('');
  const [hostname, setHostname] = useState('');
  const [publicKey, setPublicKey] = useState('');
  const [hostUserId, setHostUserId] = useState('');
  const [createdNodeToken, setCreatedNodeToken] = useState<string | null>(null);
  const [revokingNodeIds, setRevokingNodeIds] = useState<Set<string>>(() => new Set());

  const visibleItems = useMemo(() => data?.data ?? [], [data]);

  return (
    <div className="grid gap-4">
      <DataCard
        title="Daftarkan daemon"
        action={
          <button
            type="button"
            onClick={async () => {
              try {
                setCreatedNodeToken(null);
                const response = await createNode({
                  label: label.trim() || undefined,
                  hostname: hostname.trim() || undefined,
                  public_key: publicKey.trim() || undefined,
                  host_user_id: hostUserId.trim() || undefined,
                }).unwrap();
                setCreatedNodeToken(response.token);
                setLabel('');
                setHostname('');
                setPublicKey('');
                setHostUserId('');
              } catch (_error) {
                // handled via createError state rendering
              }
            }}
            disabled={creating}
            className="rounded-lg border border-shell-border px-3 py-2"
          >
            {creating ? 'Menyimpan...' : 'Buat token'}
          </button>
        }
      >
        <div className="grid gap-2 sm:grid-cols-2">
          <input
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Nama/label"
          />
          <input
            value={hostname}
            onChange={(event) => setHostname(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Hostname"
          />
          <input
            value={hostUserId}
            onChange={(event) => setHostUserId(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2"
            placeholder="Host user UUID"
          />
          <input
            value={publicKey}
            onChange={(event) => setPublicKey(event.target.value)}
            className="rounded-lg border border-shell-border bg-black/20 px-3 py-2 sm:col-span-2"
            placeholder="Public key (opsional)"
          />
        </div>
        {createError ? <ErrorText>{extractApiErrorMessage(createError, 'Gagal menyimpan node.')}</ErrorText> : null}
        {createdNodeToken ? (
          <div className="mt-3 rounded-lg border border-emerald-400/40 bg-emerald-950/30 p-3 text-xs break-all">
            <p className="mb-1 font-semibold text-emerald-300">Token node (sekali muncul):</p>
            <p className="font-mono">{createdNodeToken}</p>
            <div className="mt-2">
              <button
                type="button"
                onClick={() => {
                  if (createdNodeToken) {
                    void navigator.clipboard.writeText(createdNodeToken);
                  }
                }}
                className="rounded-lg border border-emerald-200/40 px-2 py-1 text-xs"
              >
                Salin token
              </button>
            </div>
          </div>
        ) : null}
      </DataCard>

      <DataCard title="Daemon nodes">
        <div className="mb-3 flex gap-2">
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
          isEmpty={!isLoading && !isError && visibleItems.length === 0}
          isEmptyText="Belum ada daemon terdaftar."
        />
        {revokeError ? <ErrorText>{extractApiErrorMessage(revokeError, 'Gagal mencabut node.')}</ErrorText> : null}

        {!isLoading && !isError && visibleItems.length > 0 ? (
          <ul className="space-y-2">
            {visibleItems.map((node) => {
              const isRevoking = revokingNodeIds.has(node.id);
              return (
                <li key={node.id} className="rounded-lg border border-shell-border p-3">
                  <div className="mb-2">
                    <p className="font-semibold">{node.label || 'Daemon'}</p>
                    <p className="text-xs text-slate-400">
                      {node.hostname || 'hostname'} · status {node.status} ·{' '}
                      {node.revoked ? 'revoked' : node.disabled ? 'disabled' : 'active'}
                    </p>
                    {node.capabilities.health_status ? (
                      <p className="text-[11px] text-slate-500">Health: {node.capabilities.health_status}</p>
                    ) : null}
                    <p className="font-mono text-[11px] text-slate-500">{node.id}</p>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      onClick={async () => {
                        if (isRevoking) {
                          return;
                        }
                        setRevokingNodeIds((current) => new Set(current).add(node.id));
                        try {
                          await revokeNode(node.id).unwrap();
                        } finally {
                          setRevokingNodeIds((current) => {
                            const next = new Set(current);
                            next.delete(node.id);
                            return next;
                          });
                        }
                      }}
                      disabled={isRevoking || node.revoked}
                      className="rounded-lg border border-rose-500/60 px-2 py-1 text-sm text-rose-300 disabled:opacity-40"
                    >
                      {isRevoking ? 'Mencabut...' : node.revoked ? 'Sudah revoked' : 'Revoke'}
                    </button>
                    {node.capabilities.provider_family ? (
                      <span className="rounded-lg border border-shell-border px-2 py-1 text-xs text-slate-300">
                        {node.capabilities.provider_family}
                      </span>
                    ) : null}
                    {node.capabilities.health_status ? (
                      <span className="rounded-lg border border-emerald-500/50 px-2 py-1 text-xs text-emerald-200">
                        {node.capabilities.health_status}
                      </span>
                    ) : null}
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
