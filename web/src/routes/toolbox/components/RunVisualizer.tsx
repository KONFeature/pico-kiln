import { useState, useMemo } from 'react';
import {
  LineChart,
  Line,
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
  ReferenceArea,
  ReferenceLine,
} from 'recharts';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { AlertCircle } from 'lucide-react';
import { FileSourceSelector } from './FileSourceSelector';
import { parseLogCSV, detectRunType, type LogDataPoint } from '@/lib/csv-parser';

interface ChartDataPoint {
  time_hours: number;
  time_minutes: number;
  temp: number;
  target_temp: number;
  ssr_output: number;
  step_index?: number;
  current_rate?: number;
}

export function RunVisualizer() {
  const [logData, setLogData] = useState<LogDataPoint[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [runType, setRunType] = useState<'TUNING' | 'FIRING'>('FIRING');

  const handleFileSelected = (content: string, filename: string) => {
    try {
      const { data } = parseLogCSV(content);
      
      if (data.length === 0) {
        throw new Error('No valid data points in log file');
      }

      setLogData(data);
      setRunType(detectRunType(data));
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to parse log file');
      setLogData(null);
    }
  };

  const chartData = useMemo<ChartDataPoint[]>(() => {
    if (!logData) return [];

    return logData.map(point => ({
      time_hours: point.elapsed_seconds / 3600,
      time_minutes: point.elapsed_seconds / 60,
      temp: point.current_temp_c,
      target_temp: point.target_temp_c,
      ssr_output: point.ssr_output_percent,
      step_index: point.step_index,
      current_rate: point.current_rate_c_per_hour,
    }));
  }, [logData]);

  const stats = useMemo(() => {
    if (chartData.length === 0) return null;

    const temps = chartData.map(p => p.temp);
    const maxTemp = Math.max(...temps);
    const minTemp = Math.min(...temps);
    const duration = chartData[chartData.length - 1].time_hours;
    const startTime = logData?.[0]?.timestamp || '';

    return { maxTemp, minTemp, duration, startTime };
  }, [chartData, logData]);

  const hasRateData = useMemo(() => {
    return chartData.some(p => p.current_rate !== undefined);
  }, [chartData]);

  // Calculate step transitions for visual boundaries
  const stepTransitions = useMemo(() => {
    if (!logData) return [];
    
    const transitions: number[] = [];
    let prevStep = -1;

    for (let i = 0; i < logData.length; i++) {
      const stepIdx = logData[i].step_index;
      if (stepIdx !== undefined && stepIdx !== prevStep && stepIdx >= 0 && i > 0) {
        transitions.push(logData[i].elapsed_seconds / 3600);
        prevStep = stepIdx;
      }
    }

    return transitions;
  }, [logData]);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Run Visualizer</CardTitle>
        <CardDescription>
          Visualize kiln firing or tuning runs - see temperature, SSR output, and rate data
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <FileSourceSelector
          directory="logs"
          accept=".csv"
          onFileSelected={handleFileSelected}
          label="Select Log File"
          description="Choose a log file to visualize"
        />

        {error && (
          <Alert variant="destructive">
            <AlertCircle className="h-4 w-4" />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        {logData && chartData.length > 0 && (
          <div className="space-y-6 pt-6 border-t">
            <div>
              <h3 className="text-lg font-semibold">Kiln {runType} - Temperature Profile</h3>
              {stats && (
                <p className="text-sm text-muted-foreground mt-1">
                  Started: {stats.startTime} | Duration: {stats.duration.toFixed(2)}h | 
                  Temp Range: {stats.minTemp.toFixed(1)}°C - {stats.maxTemp.toFixed(1)}°C
                </p>
              )}
            </div>

            <div className="space-y-6">
              <ResponsiveContainer width="100%" height={400}>
                <LineChart data={chartData} margin={{ top: 5, right: 30, left: 20, bottom: 5 }}>
                  <CartesianGrid strokeDasharray="3 3" opacity={0.3} />
                  {stepTransitions.map((time, idx) => (
                    <ReferenceLine
                      key={idx}
                      x={time}
                      stroke="#999"
                      strokeDasharray="3 3"
                      opacity={0.4}
                    />
                  ))}
                  <XAxis
                    dataKey="time_hours"
                    label={{ value: 'Time (hours)', position: 'insideBottom', offset: -5 }}
                  />
                  <YAxis
                    label={{ value: 'Temperature (°C)', angle: -90, position: 'insideLeft' }}
                  />
                  <Tooltip
                    formatter={(value: number, name: string) => [
                      `${value.toFixed(1)}°C`,
                      name === 'temp' ? 'Current Temp' : 'Target Temp'
                    ]}
                    labelFormatter={(label: number) => `Time: ${label.toFixed(2)}h`}
                  />
                  <Legend />
                  <Line
                    type="monotone"
                    dataKey="temp"
                    stroke="#3b82f6"
                    strokeWidth={2}
                    dot={false}
                    name="Current Temp"
                  />
                  <Line
                    type="monotone"
                    dataKey="target_temp"
                    stroke="#ef4444"
                    strokeWidth={1.5}
                    strokeDasharray="5 5"
                    dot={false}
                    opacity={0.7}
                    name="Target Temp"
                  />
                </LineChart>
              </ResponsiveContainer>

              <div className="pt-6 border-t">
                <h4 className="text-base font-semibold mb-1">SSR Output (%)</h4>
                <p className="text-sm text-muted-foreground mb-4">
                  Solid State Relay duty cycle over time
                </p>
                <ResponsiveContainer width="100%" height={200}>
                  <AreaChart data={chartData} margin={{ top: 5, right: 30, left: 20, bottom: 5 }}>
                    <CartesianGrid strokeDasharray="3 3" opacity={0.3} />
                    {stepTransitions.map((time, idx) => (
                      <ReferenceLine
                        key={idx}
                        x={time}
                        stroke="#999"
                        strokeDasharray="3 3"
                        opacity={0.4}
                      />
                    ))}
                    <XAxis
                      dataKey="time_hours"
                      label={{ value: 'Time (hours)', position: 'insideBottom', offset: -5 }}
                    />
                    <YAxis
                      domain={[0, 100]}
                      label={{ value: 'SSR Output (%)', angle: -90, position: 'insideLeft' }}
                    />
                    <Tooltip
                      formatter={(value: number) => [`${value.toFixed(1)}%`, 'SSR Output']}
                      labelFormatter={(label: number) => `Time: ${label.toFixed(2)}h`}
                    />
                    <Legend />
                    <Area
                      type="monotone"
                      dataKey="ssr_output"
                      stroke="#f97316"
                      fill="#f97316"
                      fillOpacity={0.3}
                      name="SSR Output (%)"
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>

              {hasRateData && (
                <div className="pt-6 border-t">
                  <h4 className="text-base font-semibold mb-1">Heating Rate</h4>
                  <p className="text-sm text-muted-foreground mb-4">
                    Current temperature change rate (°C/hour)
                  </p>
                  <ResponsiveContainer width="100%" height={200}>
                    <LineChart data={chartData} margin={{ top: 5, right: 30, left: 20, bottom: 5 }}>
                      <CartesianGrid strokeDasharray="3 3" opacity={0.3} />
                      {stepTransitions.map((time, idx) => (
                        <ReferenceLine
                          key={idx}
                          x={time}
                          stroke="#999"
                          strokeDasharray="3 3"
                          opacity={0.4}
                        />
                      ))}
                      <ReferenceLine y={0} stroke="#000" strokeOpacity={0.3} />
                      <XAxis
                        dataKey="time_hours"
                        label={{ value: 'Time (hours)', position: 'insideBottom', offset: -5 }}
                      />
                      <YAxis
                        label={{ value: 'Rate (°C/h)', angle: -90, position: 'insideLeft' }}
                      />
                      <Tooltip
                        formatter={(value: number) => [`${value.toFixed(1)}°C/h`, 'Rate']}
                        labelFormatter={(label: number) => `Time: ${label.toFixed(2)}h`}
                      />
                      <Legend />
                      <Line
                        type="monotone"
                        dataKey="current_rate"
                        stroke="#22c55e"
                        strokeWidth={2}
                        dot={false}
                        name="Current Rate (°C/h)"
                      />
                    </LineChart>
                  </ResponsiveContainer>
                </div>
              )}

              <div className="pt-6 border-t">
                <h4 className="text-base font-semibold mb-4">Run Statistics</h4>
                <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                  <div>
                    <div className="text-muted-foreground">Run Type</div>
                    <div className="font-semibold text-lg">{runType}</div>
                  </div>
                  <div>
                    <div className="text-muted-foreground">Duration</div>
                    <div className="font-semibold text-lg">{stats?.duration.toFixed(2)}h</div>
                  </div>
                  <div>
                    <div className="text-muted-foreground">Max Temperature</div>
                    <div className="font-semibold text-lg">{stats?.maxTemp.toFixed(1)}°C</div>
                  </div>
                  <div>
                    <div className="text-muted-foreground">Data Points</div>
                    <div className="font-semibold text-lg">{chartData.length.toLocaleString()}</div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
