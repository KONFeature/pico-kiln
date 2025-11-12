// Connection status indicator component

import { usePico } from '@/lib/pico/context';
import { Badge } from '@/components/ui/badge';
import { Wifi, WifiOff, Loader2 } from 'lucide-react';
import { useKilnStatus } from '@/lib/pico/hooks';

export function ConnectionStatus() {
  const { isConfigured, connectionHealth, picoURL } = usePico();
  const { isLoading } = useKilnStatus({ enabled: isConfigured });

  if (!isConfigured) {
    return (
      <Badge variant="outline" className="flex items-center gap-2">
        <WifiOff className="w-3 h-3" />
        <span>Not Connected</span>
      </Badge>
    );
  }

  if (isLoading && connectionHealth.consecutiveFailures === 0) {
    return (
      <Badge variant="outline" className="flex items-center gap-2">
        <Loader2 className="w-3 h-3 animate-spin" />
        <span>Connecting...</span>
      </Badge>
    );
  }

  if (connectionHealth.connected) {
    return (
      <Badge variant="default" className="flex items-center gap-2 bg-green-600 hover:bg-green-700">
        <Wifi className="w-3 h-3" />
        <span>Connected</span>
      </Badge>
    );
  }

  // Show error state if we have consecutive failures
  if (connectionHealth.consecutiveFailures > 0) {
    return (
      <Badge variant="destructive" className="flex items-center gap-2">
        <WifiOff className="w-3 h-3" />
        <span>Connection Error</span>
      </Badge>
    );
  }

  return (
    <Badge variant="outline" className="flex items-center gap-2">
      <WifiOff className="w-3 h-3" />
      <span>Unknown</span>
    </Badge>
  );
}

export function ConnectionStatusDetailed() {
  const { isConfigured, connectionHealth, picoURL } = usePico();

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium">Connection Status:</span>
        <ConnectionStatus />
      </div>
      
      {isConfigured && (
        <div className="text-xs text-muted-foreground space-y-1">
          <div>URL: {picoURL}</div>
          {connectionHealth.lastSuccessfulRequest && (
            <div>
              Last success: {new Date(connectionHealth.lastSuccessfulRequest).toLocaleTimeString()}
            </div>
          )}
          {connectionHealth.consecutiveFailures > 0 && (
            <div className="text-destructive">
              Failed attempts: {connectionHealth.consecutiveFailures}
            </div>
          )}
          {connectionHealth.lastError && (
            <div className="text-destructive">
              Error: {connectionHealth.lastError}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
