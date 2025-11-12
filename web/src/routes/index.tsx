import { createFileRoute } from '@tanstack/react-router'
import { RequireConnection } from '@/components/RequireConnection'
import { KilnStatusDisplay } from '@/routes/control/components/KilnStatusDisplay'
import { ProfileControls } from '@/routes/control/components/ProfileControls'
import { TuningControls } from '@/routes/control/components/TuningControls'
import { useKilnStatus } from '@/lib/pico/hooks'

export const Route = createFileRoute('/')({
  component: HomePage,
})

function HomePage() {
  const { data: status, isLoading, error } = useKilnStatus()

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
            />
          </div>

          <div className="space-y-6">
            {status?.state === 'TUNING' ? (
              <TuningControls status={status} />
            ) : (
              <ProfileControls status={status} />
            )}
          </div>
        </div>
      </div>
    </RequireConnection>
  )
}
