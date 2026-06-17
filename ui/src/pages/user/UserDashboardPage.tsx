import { useGetWalletQuery } from '@/features/api/mizanApi';
import { DataCard } from '@/components/DataRow';

export function UserDashboardPage() {
  const { data: wallet, isLoading: walletLoading, isError: walletError } = useGetWalletQuery();

  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <DataCard title="Saldo credit">
        {walletLoading ? (
          <p>Loading...</p>
        ) : walletError ? (
          <p>Gagal load saldo.</p>
        ) : (
          <p className="text-2xl font-semibold">{wallet?.balance_microcredits.toLocaleString()} mc</p>
        )}
      </DataCard>

      <DataCard title="Status akun">
        <p className="text-sm text-slate-300">Token API virtual aktif dan siap dipakai.</p>
      </DataCard>
    </div>
  );
}
