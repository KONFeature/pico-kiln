import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Loader2, Thermometer, Flame, AlertTriangle, Clock, Gauge, TrendingUp, AlertCircle } from 'lucide-react'
import type { KilnStatus } from '@/lib/pico/types'
import type { PicoAPIError } from '@/lib/pico/client'

interface KilnStatusDisplayProps {
  status?: KilnStatus
  isLoading: boolean
  error: PicoAPIError | null
}

export function KilnStatusDisplay({ status, isLoading, error }: KilnStatusDisplayProps) {
  if (error) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Kiln Status</CardTitle>
        </CardHeader>
        <CardContent>
          <Alert variant="destructive">
            <AlertTriangle className="w-4 h-4" />
            <AlertDescription>
              Failed to load kiln status: {error.message}
            </AlertDescription>
          </Alert>
        </CardContent>
      </Card>
    )
  }

  if (isLoading && !status) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Kiln Status</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-center py-8">
            <Loader2 className="w-8 h-8 animate-spin text-muted-foreground" />
          </div>
        </CardContent>
      </Card>
    )
  }

  if (!status) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Kiln Status</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-muted-foreground">No status data available</p>
        </CardContent>
      </Card>
    )
  }

  const getStateBadge = (state: KilnStatus['state']) => {
    switch (state) {
      case 'IDLE':
        return <Badge variant="outline">Idle</Badge>
      case 'RUNNING':
        return <Badge className="bg-blue-600 hover:bg-blue-700">Running</Badge>
      case 'TUNING':
        return <Badge className="bg-purple-600 hover:bg-purple-700">Tuning</Badge>
      case 'ERROR':
        return <Badge variant="destructive">Error</Badge>
      default:
        return <Badge variant="outline">{state}</Badge>
    }
  }

  const formatTemp = (temp: number) => {
    return `${temp.toFixed(1)}°C`
  }

  const formatDuration = (seconds?: number) => {
    if (!seconds) return 'N/A'
    const hours = Math.floor(seconds / 3600)
    const minutes = Math.floor((seconds % 3600) / 60)
    const secs = Math.floor(seconds % 60)
    
    if (hours > 0) {
      return `${hours}h ${minutes}m`
    }
    if (minutes > 0) {
      return `${minutes}m ${secs}s`
    }
    return `${secs}s`
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>Kiln Status</CardTitle>
            {getStateBadge(status.state)}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Temperature Display */}
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1">
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Thermometer className="w-4 h-4" />
                Current Temperature
              </div>
              <div className="text-2xl font-bold">
                {formatTemp(status.current_temp)}
              </div>
            </div>

            {status.target_temp !== undefined && (
              <div className="space-y-1">
                <div className="text-sm text-muted-foreground">
                  Target Temperature
                </div>
                <div className="text-2xl font-bold">
                  {formatTemp(status.target_temp)}
                </div>
              </div>
            )}
          </div>

          {/* SSR and Heating Rates */}
          <div className="grid grid-cols-2 gap-4 pt-2 border-t">
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <Flame className={`w-5 h-5 ${(status.ssr_output ?? 0) > 0 ? 'text-orange-500' : 'text-gray-400'}`} />
                <span className="text-sm font-medium">
                  SSR Output
                </span>
              </div>
              <div className="flex items-center gap-2">
                <Gauge className="w-4 h-4 text-muted-foreground" />
                <span className="text-sm font-bold">
                  {status.ssr_output !== undefined 
                    ? status.ssr_output > 0 
                      ? `${status.ssr_output.toFixed(1)}%`
                      : 'OFF'
                    : 'N/A'
                  }
                </span>
              </div>
            </div>

            {(status.actual_rate !== undefined || status.current_rate !== undefined) && (
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <TrendingUp className="w-5 h-5 text-muted-foreground" />
                  <span className="text-sm font-medium">
                    Heating Rate
                  </span>
                </div>
                {status.actual_rate !== undefined && (
                  <div className="text-sm">
                    Actual: <strong>{status.actual_rate.toFixed(1)}°C/h</strong>
                  </div>
                )}
                {status.current_rate !== undefined && status.state === 'RUNNING' && (
                  <div className="text-sm text-muted-foreground">
                    Target: {status.current_rate.toFixed(1)}°C/h
                  </div>
                )}
              </div>
            )}
          </div>

          {/* Recovery Mode Warning */}
          {status.is_recovering && (
            <Alert className="border-orange-500 bg-orange-50 dark:bg-orange-950/20">
              <AlertCircle className="h-4 w-4 text-orange-600" />
              <AlertDescription className="text-orange-800 dark:text-orange-400">
                <strong>Recovery Mode:</strong> Kiln is recovering from temperature drop.
                {status.recovery_target_temp !== undefined && (
                  <> Target: {formatTemp(status.recovery_target_temp)}</>
                )}
              </AlertDescription>
            </Alert>
          )}

          {status.error_message && (
            <Alert variant="destructive">
              <AlertTriangle className="w-4 h-4" />
              <AlertDescription>{status.error_message}</AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      {status.state === 'RUNNING' && status.profile_name && (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle>Profile Progress</CardTitle>
              {status.step_index !== undefined && status.total_steps !== undefined && (
                <Badge variant="outline" className="text-sm">
                  Step {status.step_index + 1} / {status.total_steps}
                </Badge>
              )}
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            {/* Profile Name */}
            <div className="space-y-1">
              <div className="text-sm text-muted-foreground">Active Profile</div>
              <div className="text-xl font-bold">{status.profile_name}</div>
            </div>

            {/* Current Step Info */}
            {status.step_name && (
              <div className="p-3 rounded-lg bg-muted/50 space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-sm font-medium">
                    Current Step: {status.step_name}
                  </span>
                  {status.desired_rate !== undefined && status.step_name === 'ramp' && (
                    <span className="text-sm text-muted-foreground">
                      {status.desired_rate.toFixed(0)}°C/h
                    </span>
                  )}
                </div>
              </div>
            )}

            {/* Time Information */}
            {status.elapsed !== undefined && (
              <div className="grid grid-cols-1 gap-4 text-sm pt-2 border-t">
                <div>
                  <div className="text-muted-foreground">Elapsed Time</div>
                  <div className="font-medium flex items-center gap-1 mt-1">
                    <Clock className="w-4 h-4" />
                    {formatDuration(status.elapsed)}
                  </div>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {status.pid && (
        <Card>
          <CardHeader>
            <CardTitle>PID Control</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-4 text-sm">
              {status.pid.kp !== undefined && (
                <div>
                  <div className="text-muted-foreground">Kp</div>
                  <div className="font-mono">{status.pid.kp.toFixed(3)}</div>
                </div>
              )}
              {status.pid.ki !== undefined && (
                <div>
                  <div className="text-muted-foreground">Ki</div>
                  <div className="font-mono">{status.pid.ki.toFixed(3)}</div>
                </div>
              )}
              {status.pid.kd !== undefined && (
                <div>
                  <div className="text-muted-foreground">Kd</div>
                  <div className="font-mono">{status.pid.kd.toFixed(3)}</div>
                </div>
              )}
              {status.pid.output !== undefined && (
                <div>
                  <div className="text-muted-foreground">Output</div>
                  <div className="font-mono">{status.pid.output.toFixed(1)}%</div>
                </div>
              )}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
