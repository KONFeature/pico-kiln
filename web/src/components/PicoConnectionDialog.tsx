// Dialog for editing Pico connection settings

import { useState, useEffect } from 'react';
import { usePico } from '@/lib/pico/context';
import { useTestConnection } from '@/lib/pico/hooks';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Alert, AlertDescription } from '@/components/ui/alert';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Loader2, CheckCircle2, XCircle, Wifi } from 'lucide-react';

interface PicoConnectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function PicoConnectionDialog({ open, onOpenChange }: PicoConnectionDialogProps) {
  const { picoURL, setPicoURL } = usePico();
  const testConnection = useTestConnection();
  
  const [inputURL, setInputURL] = useState(picoURL || 'http://');
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);

  // Reset input when dialog opens
  useEffect(() => {
    if (open) {
      setInputURL(picoURL || 'http://');
      setTestResult(null);
    }
  }, [open, picoURL]);

  const handleSave = () => {
    const trimmedURL = inputURL.trim();
    if (trimmedURL && trimmedURL !== 'http://' && trimmedURL !== 'https://') {
      setPicoURL(trimmedURL);
      setTestResult(null);
    }
  };

  const handleTest = async () => {
    // Save first if URL changed
    const trimmedURL = inputURL.trim();
    if (trimmedURL !== picoURL) {
      handleSave();
    }
    
    setTestResult(null);
    const result = await testConnection.mutateAsync();
    setTestResult(result ? 'success' : 'error');
  };

  const handleReset = () => {
    setPicoURL('');
    setInputURL('http://');
    setTestResult(null);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Pico Connection Settings</DialogTitle>
          <DialogDescription>
            Configure the connection to your Pico kiln controller
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <div className="space-y-2">
            <Label htmlFor="pico-url-dialog">Pico URL</Label>
            <Input
              id="pico-url-dialog"
              type="text"
              placeholder="http://192.168.1.100:80"
              value={inputURL}
              onChange={(e) => setInputURL(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  handleSave();
                }
              }}
            />
            <p className="text-xs text-muted-foreground">
              Example: http://192.168.1.100:80 or http://pico-kiln.local:80
            </p>
          </div>

          {picoURL && (
            <div className="text-sm text-muted-foreground">
              Current URL: <span className="font-mono">{picoURL}</span>
            </div>
          )}

          {testResult === 'success' && (
            <Alert className="border-green-600 bg-green-50">
              <CheckCircle2 className="w-4 h-4 text-green-600" />
              <AlertDescription className="text-green-800">
                Connection successful!
              </AlertDescription>
            </Alert>
          )}

          {testResult === 'error' && (
            <Alert variant="destructive">
              <XCircle className="w-4 h-4" />
              <AlertDescription>
                Connection failed. Please check the URL and ensure the Pico is on the same network.
              </AlertDescription>
            </Alert>
          )}
        </div>

        <DialogFooter className="flex-col sm:flex-row gap-2">
          <div className="flex gap-2 flex-1">
            <Button 
              onClick={handleTest} 
              disabled={testConnection.isPending}
              variant="outline"
              className="flex-1"
            >
              {testConnection.isPending ? (
                <>
                  <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                  Testing...
                </>
              ) : (
                <>
                  <Wifi className="w-4 h-4 mr-2" />
                  Test
                </>
              )}
            </Button>
            {picoURL && (
              <Button onClick={handleReset} variant="outline">
                Reset
              </Button>
            )}
          </div>
          <Button onClick={handleSave} className="flex-1 sm:flex-initial">
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
