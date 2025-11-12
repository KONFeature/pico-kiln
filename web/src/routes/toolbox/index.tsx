import { createFileRoute } from '@tanstack/react-router'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Wrench } from 'lucide-react'

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

      <Card>
        <CardHeader>
          <CardTitle>Profile Tools</CardTitle>
          <CardDescription>
            Visualize and create custom firing profiles
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-muted-foreground">
            Profile visualization and creation tools will be implemented here.
          </p>
          <p className="text-sm text-muted-foreground mt-2">
            Coming soon: Visual profile editor, profile analysis, and profile comparison tools.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
