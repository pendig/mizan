export function extractApiErrorMessage(error: unknown, fallback: string) {
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

  return fallback;
}
