import { createFileRoute } from '@tanstack/react-router'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Wrench, LineChart, Activity, Zap, Edit } from 'lucide-react'
import { ProfileVisualizer } from './components/ProfileVisualizer'
import { RunVisualizer } from './components/RunVisualizer'
import { TuningPhasesVisualizer } from './components/TuningPhasesVisualizer'
import { ProfileEditor } from './components/ProfileEditor'

export const Route = createFileRoute('/toolbox/')({
  component: ToolboxPage,
})

function ToolboxPage() {
  return (
    <div className="container max-w-7xl mx-auto py-8 px-4 space-y-6">
        <div className="flex items-center gap-3">
          <Wrench className="w-8 h-8 text-green-500" />
          <h1 className="text-3xl font-bold">Toolbox</h1>
        </div>

        <p className="text-muted-foreground">
          Visualize profiles and runs, analyze tuning data, and create custom firing profiles.
          Upload files or load them directly from your Pico when it's IDLE.
        </p>

        <Tabs defaultValue="profile-viz" className="w-full">
          <TabsList className="grid w-full grid-cols-4">
            <TabsTrigger value="profile-viz" className="flex items-center gap-2">
              <LineChart className="w-4 h-4" />
              Profile Viz
            </TabsTrigger>
            <TabsTrigger value="run-viz" className="flex items-center gap-2">
              <Activity className="w-4 h-4" />
              Run Viz
            </TabsTrigger>
            <TabsTrigger value="tuning-viz" className="flex items-center gap-2">
              <Zap className="w-4 h-4" />
              Tuning Phases
            </TabsTrigger>
            <TabsTrigger value="editor" className="flex items-center gap-2">
              <Edit className="w-4 h-4" />
              Profile Editor
            </TabsTrigger>
          </TabsList>

          <TabsContent value="profile-viz" className="mt-6">
            <ProfileVisualizer />
          </TabsContent>

          <TabsContent value="run-viz" className="mt-6">
            <RunVisualizer />
          </TabsContent>

          <TabsContent value="tuning-viz" className="mt-6">
            <TuningPhasesVisualizer />
          </TabsContent>

          <TabsContent value="editor" className="mt-6">
            <ProfileEditor />
          </TabsContent>
        </Tabs>
      </div>
  )
}
