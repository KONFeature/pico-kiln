import { createFileRoute } from '@tanstack/react-router'
import { RequireConnection } from '@/components/RequireConnection'
import { KilnStatusDisplay } from '@/routes/control/components/KilnStatusDisplay'
import { ProfileControls } from '@/routes/control/components/ProfileControls'
import { TuningControls } from '@/routes/control/components/TuningControls'
import { useKilnStatus } from '@/lib/pico/hooks'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'
import { Flame, Zap } from 'lucide-react'

export const Route = createFileRoute('/')({
  component: HomePage,
})

function HomePage() {
  const { data: status, isLoading, error, refetch } = useKilnStatus()

  // Auto-select tab based on current state
  const defaultTab = status?.state === 'TUNING' ? 'tuning' : 'profile'

  return (
    <RequireConnection>
      <div className="container max-w-7xl mx-auto py-8 px-4 space-y-6">
        <div className="flex items-center justify-between">
          <h1 className="text-3xl font-bold">Kiln Control</h1>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <div className="space-y-6">
            <KilnStatusDisplay 
              status={status} 
              isLoading={isLoading} 
              error={error}
              onRefresh={() => refetch()}
            />
          </div>

          <div className="space-y-6">
            <Tabs defaultValue={defaultTab} key={defaultTab}>
              <TabsList className="w-full grid grid-cols-2">
                <TabsTrigger value="profile" disabled={status?.state === 'TUNING'}>
                  <Flame className="w-4 h-4 mr-2" />
                  Run Profile
                </TabsTrigger>
                <TabsTrigger value="tuning" disabled={status?.state === 'RUNNING'}>
                  <Zap className="w-4 h-4 mr-2" />
                  PID Tuning
                </TabsTrigger>
              </TabsList>
              
              <TabsContent value="profile">
                <ProfileControls status={status} />
              </TabsContent>
              
              <TabsContent value="tuning">
                <TuningControls status={status} />
              </TabsContent>
            </Tabs>
          </div>
        </div>
      </div>
    </RequireConnection>
  )
}
