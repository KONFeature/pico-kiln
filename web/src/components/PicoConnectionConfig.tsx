// Pico connection configuration component

import { useState } from 'react';
import { usePico } from '@/lib/pico/context';
import { useTestConnection } from '@/lib/pico/hooks';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { ConnectionStatusDetailed } from './ConnectionStatus';
import { Loader2, CheckCircle2, XCircle } from 'lucide-react';

interface PicoConnectionConfigProps {
  onConnected?: () => void;
}

export function PicoConnectionConfig({ onConnected }: PicoConnectionConfigProps) {
  const { picoURL, setPicoURL, isConfigured } = usePico();
  const testConnection = useTestConnection();
  
  const [inputURL, setInputURL] = useState(picoURL || 'http://');
  const [testResult, setTestResult] = useState<'success' | 'error' | null>(null);

  const handleSave = () => {
    const trimmedURL = inputURL.trim();
    if (trimmedURL && trimmedURL !== 'http://' && trimmedURL !== 'https://') {
      setPicoURL(trimmedURL);
      setTestResult(null);
    }
  };

  const handleTest = async () => {
    setTestResult(null);
    const result = await testConnection.mutateAsync();
    setTestResult(result ? 'success' : 'error');
    
    if (result && onConnected) {
      setTimeout(onConnected, 1000);
    }
  };

  const handleReset = () => {
    setPicoURL('');
    setInputURL('http://');
    setTestResult(null);
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Pico Kiln Connection</CardTitle>
        <CardDescription>
          Enter the IP address and port of your Pico kiln controller
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <Label htmlFor="pico-url">Pico URL</Label>
          <div className="flex gap-2">
            <Input
              id="pico-url"
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
            <Button onClick={handleSave} variant="secondary">
              Save
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            Example: http://192.168.1.100:80 or http://pico-kiln.local:80
          </p>
        </div>

        {isConfigured && (
          <>
            <div className="space-y-2">
              <ConnectionStatusDetailed />
            </div>

            <div className="flex gap-2">
              <Button 
                onClick={handleTest} 
                disabled={testConnection.isPending}
                className="flex-1"
              >
                {testConnection.isPending ? (
                  <>
                    <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                    Testing...
                  </>
                ) : (
                  'Test Connection'
                )}
              </Button>
              <Button onClick={handleReset} variant="outline">
                Reset
              </Button>
            </div>

            {testResult === 'success' && (
              <Alert className="border-green-600 bg-green-50">
                <CheckCircle2 className="w-4 h-4 text-green-600" />
                <AlertDescription className="text-green-800">
                  Connection successful! You can now control your kiln.
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
          </>
        )}
      </CardContent>
    </Card>
  );
}
