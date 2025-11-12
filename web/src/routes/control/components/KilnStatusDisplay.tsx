import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Loader2, Thermometer, Flame, AlertTriangle, Clock } from 'lucide-react'
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

          <div className="flex items-center gap-2">
            <Flame className={`w-5 h-5 ${status.ssr_on ? 'text-orange-500' : 'text-gray-400'}`} />
            <span className="text-sm">
              Heating Element: <strong>{status.ssr_on ? 'ON' : 'OFF'}</strong>
            </span>
          </div>

          {status.error_message && (
            <Alert variant="destructive">
              <AlertTriangle className="w-4 h-4" />
              <AlertDescription>{status.error_message}</AlertDescription>
            </Alert>
          )}
        </CardContent>
      </Card>

      {status.profile && (
        <Card>
          <CardHeader>
            <CardTitle>Profile Progress</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-2">
              <div className="text-lg font-semibold">{status.profile.profile_name}</div>
              <div className="text-sm text-muted-foreground">
                Step {status.profile.current_step} of {status.profile.total_steps} ({status.profile.step_type})
              </div>
            </div>

            {status.profile.step_progress !== undefined && (
              <div className="space-y-2">
                <div className="flex items-center justify-between text-sm">
                  <span>Step Progress</span>
                  <span className="font-medium">{status.profile.step_progress.toFixed(1)}%</span>
                </div>
                <div className="w-full bg-gray-200 rounded-full h-2">
                  <div 
                    className="bg-blue-600 h-2 rounded-full transition-all duration-500"
                    style={{ width: `${status.profile.step_progress}%` }}
                  />
                </div>
              </div>
            )}

            <div className="grid grid-cols-2 gap-4 text-sm">
              {status.profile.elapsed_time !== undefined && (
                <div>
                  <div className="text-muted-foreground">Elapsed Time</div>
                  <div className="font-medium flex items-center gap-1">
                    <Clock className="w-4 h-4" />
                    {formatDuration(status.profile.elapsed_time)}
                  </div>
                </div>
              )}
              {status.profile.estimated_time_remaining !== undefined && (
                <div>
                  <div className="text-muted-foreground">Time Remaining</div>
                  <div className="font-medium flex items-center gap-1">
                    <Clock className="w-4 h-4" />
                    {formatDuration(status.profile.estimated_time_remaining)}
                  </div>
                </div>
              )}
            </div>
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
