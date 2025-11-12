import { useState } from 'react'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { useRunProfile, useStopProfile, useShutdown } from '@/lib/pico/hooks'
import { Loader2, Play, Square, AlertTriangle, Power } from 'lucide-react'
import type { KilnStatus } from '@/lib/pico/types'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

interface ProfileControlsProps {
  status?: KilnStatus
}

// NOTE: In the future, this list should come from an API endpoint
// For now, we'll use a hardcoded list based on the profiles directory
const AVAILABLE_PROFILES = [
  'biscuit_faience_adaptive',
  'test_adaptive',
]

export function ProfileControls({ status }: ProfileControlsProps) {
  const [selectedProfile, setSelectedProfile] = useState<string>('')
  const [showShutdownDialog, setShowShutdownDialog] = useState(false)
  
  const runProfile = useRunProfile()
  const stopProfile = useStopProfile()
  const shutdown = useShutdown()

  const isRunning = status?.state === 'RUNNING'
  const canStart = !isRunning && selectedProfile && status?.state !== 'TUNING'
  const canStop = isRunning

  const handleRun = async () => {
    if (!selectedProfile) return
    
    try {
      const result = await runProfile.mutateAsync(selectedProfile)
      if (!result.success) {
        console.error('Failed to start profile:', result.error)
      }
    } catch (error) {
      console.error('Error starting profile:', error)
    }
  }

  const handleStop = async () => {
    try {
      const result = await stopProfile.mutateAsync()
      if (!result.success) {
        console.error('Failed to stop profile:', result.error)
      }
    } catch (error) {
      console.error('Error stopping profile:', error)
    }
  }

  const handleShutdown = async () => {
    try {
      const result = await shutdown.mutateAsync()
      if (result.success) {
        setShowShutdownDialog(false)
      }
    } catch (error) {
      console.error('Error during shutdown:', error)
    }
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Profile Control</CardTitle>
          <CardDescription>
            Select and run a firing profile
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {!isRunning ? (
            <>
              <div className="space-y-2">
                <label className="text-sm font-medium">Select Profile</label>
                <select
                  className="w-full px-3 py-2 border rounded-md bg-background"
                  value={selectedProfile}
                  onChange={(e) => setSelectedProfile(e.target.value)}
                  disabled={status?.state === 'TUNING'}
                >
                  <option value="">-- Choose a profile --</option>
                  {AVAILABLE_PROFILES.map((profile) => (
                    <option key={profile} value={profile}>
                      {profile.replace(/_/g, ' ')}
                    </option>
                  ))}
                </select>
              </div>

              <Button
                onClick={handleRun}
                disabled={!canStart || runProfile.isPending}
                className="w-full"
                size="lg"
              >
                {runProfile.isPending ? (
                  <>
                    <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                    Starting...
                  </>
                ) : (
                  <>
                    <Play className="w-4 h-4 mr-2" />
                    Start Profile
                  </>
                )}
              </Button>

              {runProfile.isError && (
                <Alert variant="destructive">
                  <AlertTriangle className="w-4 h-4" />
                  <AlertDescription>
                    {runProfile.error?.message || 'Failed to start profile'}
                  </AlertDescription>
                </Alert>
              )}
            </>
          ) : (
            <>
              <Alert className="border-blue-600 bg-blue-50">
                <AlertDescription className="text-blue-800">
                  Profile is currently running: <strong>{status?.profile?.profile_name}</strong>
                </AlertDescription>
              </Alert>

              <Button
                onClick={handleStop}
                disabled={!canStop || stopProfile.isPending}
                variant="destructive"
                className="w-full"
                size="lg"
              >
                {stopProfile.isPending ? (
                  <>
                    <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                    Stopping...
                  </>
                ) : (
                  <>
                    <Square className="w-4 h-4 mr-2" />
                    Stop Profile
                  </>
                )}
              </Button>

              {stopProfile.isError && (
                <Alert variant="destructive">
                  <AlertTriangle className="w-4 h-4" />
                  <AlertDescription>
                    {stopProfile.error?.message || 'Failed to stop profile'}
                  </AlertDescription>
                </Alert>
              )}
            </>
          )}
        </CardContent>
      </Card>

      <Card className="border-red-600">
        <CardHeader>
          <CardTitle className="text-destructive">Emergency Controls</CardTitle>
          <CardDescription>
            Use with caution - immediately stops all heating
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button
            onClick={() => setShowShutdownDialog(true)}
            variant="destructive"
            className="w-full"
            size="lg"
          >
            <Power className="w-4 h-4 mr-2" />
            Emergency Shutdown
          </Button>
        </CardContent>
      </Card>

      <Dialog open={showShutdownDialog} onOpenChange={setShowShutdownDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Confirm Emergency Shutdown</DialogTitle>
            <DialogDescription>
              This will immediately turn off the heating element and stop any running program.
              The kiln will begin cooling naturally.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowShutdownDialog(false)}
              disabled={shutdown.isPending}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleShutdown}
              disabled={shutdown.isPending}
            >
              {shutdown.isPending ? (
                <>
                  <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                  Shutting down...
                </>
              ) : (
                'Confirm Shutdown'
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
