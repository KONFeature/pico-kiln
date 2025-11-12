import { useState } from 'react';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Alert, AlertDescription } from '@/components/ui/alert';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Folder,
  File,
  Download,
  Trash2,
  Upload,
  RefreshCw,
  AlertCircle,
  CheckCircle,
  Loader2,
} from 'lucide-react';
import { usePico } from '@/lib/pico/context';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type { FileDirectory } from '@/lib/pico/types';

interface FileItem {
  name: string;
  size: number;
}

export function FileManager() {
  const [selectedDirectory, setSelectedDirectory] = useState<FileDirectory>('profiles');
  const [fileToDelete, setFileToDelete] = useState<{ directory: FileDirectory; filename: string } | null>(null);
  const [showDeleteAllDialog, setShowDeleteAllDialog] = useState(false);
  const [uploadFile, setUploadFile] = useState<File | null>(null);
  const [uploadDirectory, setUploadDirectory] = useState<FileDirectory>('profiles');
  const [showUploadDialog, setShowUploadDialog] = useState(false);
  const { client, isConfigured } = usePico();
  const queryClient = useQueryClient();

  // Query to list files
  const {
    data: filesData,
    isLoading: isLoadingFiles,
    error: filesError,
    refetch: refetchFiles,
  } = useQuery({
    queryKey: ['files', selectedDirectory],
    queryFn: () => client!.listFiles(selectedDirectory),
    enabled: isConfigured && client !== null,
    retry: 1,
  });

  // Mutation for deleting a file
  const deleteMutation = useMutation({
    mutationFn: ({ directory, filename }: { directory: FileDirectory; filename: string }) => {
      if (!client) throw new Error('Pico client not configured');
      return client.deleteFile(directory, filename);
    },
    onSuccess: (data, variables) => {
      if (data.success) {
        // Invalidate the file list to refresh
        queryClient.invalidateQueries({ queryKey: ['files', variables.directory] });
        setFileToDelete(null);
      }
    },
  });

  // Mutation for deleting all logs
  const deleteAllLogsMutation = useMutation({
    mutationFn: () => {
      if (!client) throw new Error('Pico client not configured');
      return client.deleteAllLogs();
    },
    onSuccess: (data) => {
      if (data.success) {
        queryClient.invalidateQueries({ queryKey: ['files', 'logs'] });
        setShowDeleteAllDialog(false);
      }
    },
  });

  // Mutation for uploading a file
  const uploadMutation = useMutation({
    mutationFn: async ({ directory, filename, content }: { directory: FileDirectory; filename: string; content: string }) => {
      if (!client) throw new Error('Pico client not configured');
      return client.uploadFile(directory, filename, content);
    },
    onSuccess: (data, variables) => {
      if (data.success) {
        queryClient.invalidateQueries({ queryKey: ['files', variables.directory] });
        setShowUploadDialog(false);
        setUploadFile(null);
      }
    },
  });

  const handleDownloadFile = async (directory: FileDirectory, filename: string) => {
    if (!client) return;

    try {
      const response = await client.getFile(directory, filename);
      if (response.success && response.content) {
        // Create a blob and download
        const blob = new Blob([response.content], { type: 'text/plain' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = filename;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      }
    } catch (error) {
      console.error('Failed to download file:', error);
    }
  };

  const handleUploadFile = () => {
    if (!uploadFile) return;

    const reader = new FileReader();
    reader.onload = (event) => {
      let content = event.target?.result as string;
      
      // Minify JSON files to save space on Pico
      if (uploadFile.name.endsWith('.json')) {
        try {
          const parsed = JSON.parse(content);
          content = JSON.stringify(parsed); // Minified (no indentation)
        } catch (e) {
          // If JSON parsing fails, upload as-is
          console.warn('Failed to minify JSON, uploading as-is:', e);
        }
      }
      
      uploadMutation.mutate({
        directory: uploadDirectory,
        filename: uploadFile.name,
        content,
      });
    };
    reader.readAsText(uploadFile);
  };

  const formatFileSize = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  const files = filesData?.success ? filesData.files : [];
  const totalSize = files.reduce((sum, file) => sum + file.size, 0);

  return (
    <Card>
      <CardHeader>
        <CardTitle>File Manager</CardTitle>
        <CardDescription>
          Browse, download, upload, and delete files from your Pico. Only works when kiln is IDLE.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {!isConfigured && (
          <Alert>
            <AlertCircle className="h-4 w-4" />
            <AlertDescription>
              Please configure Pico connection to manage files
            </AlertDescription>
          </Alert>
        )}

        {isConfigured && (
          <>
            {/* Directory Selection */}
            <div>
              <Label className="text-base">Select Directory</Label>
              <div className="flex gap-2 mt-2">
                <Button
                  variant={selectedDirectory === 'profiles' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setSelectedDirectory('profiles')}
                  className="flex-1"
                >
                  <Folder className="w-4 h-4 mr-2" />
                  Profiles
                </Button>
                <Button
                  variant={selectedDirectory === 'logs' ? 'default' : 'outline'}
                  size="sm"
                  onClick={() => setSelectedDirectory('logs')}
                  className="flex-1"
                >
                  <Folder className="w-4 h-4 mr-2" />
                  Logs
                </Button>
              </div>
            </div>

            {/* Actions */}
            <div className="flex gap-2 items-center justify-between pt-4 border-t">
              <div className="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => refetchFiles()}
                  disabled={isLoadingFiles}
                >
                  <RefreshCw className={`w-4 h-4 mr-2 ${isLoadingFiles ? 'animate-spin' : ''}`} />
                  Refresh
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setUploadDirectory(selectedDirectory);
                    setShowUploadDialog(true);
                  }}
                >
                  <Upload className="w-4 h-4 mr-2" />
                  Upload
                </Button>
              </div>

              {selectedDirectory === 'logs' && files.length > 0 && (
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={() => setShowDeleteAllDialog(true)}
                >
                  <Trash2 className="w-4 h-4 mr-2" />
                  Delete All Logs
                </Button>
              )}
            </div>

            {/* File List */}
            <div className="space-y-2">
              {filesError && (
                <Alert variant="destructive">
                  <AlertCircle className="h-4 w-4" />
                  <AlertDescription>
                    Failed to load files. Make sure the kiln is IDLE.
                  </AlertDescription>
                </Alert>
              )}

              {isLoadingFiles && (
                <div className="flex items-center justify-center py-8 text-muted-foreground">
                  <Loader2 className="w-5 h-5 mr-2 animate-spin" />
                  Loading files...
                </div>
              )}

              {!isLoadingFiles && files.length === 0 && (
                <div className="text-center py-8 text-muted-foreground">
                  No files found in {selectedDirectory} directory
                </div>
              )}

              {!isLoadingFiles && files.length > 0 && (
                <>
                  <div className="text-sm text-muted-foreground mb-2">
                    {files.length} file{files.length !== 1 ? 's' : ''} • {formatFileSize(totalSize)} total
                  </div>
                  <div className="space-y-1">
                    {files.map((file) => (
                      <div
                        key={file.name}
                        className="flex items-center justify-between p-3 rounded-lg border bg-card hover:bg-muted/50 transition-colors"
                      >
                        <div className="flex items-center gap-3 flex-1 min-w-0">
                          <File className="w-4 h-4 text-muted-foreground flex-shrink-0" />
                          <div className="min-w-0 flex-1">
                            <div className="font-medium truncate">{file.name}</div>
                            <div className="text-xs text-muted-foreground">
                              {formatFileSize(file.size)}
                            </div>
                          </div>
                        </div>
                        <div className="flex items-center gap-1 flex-shrink-0">
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => handleDownloadFile(selectedDirectory, file.name)}
                          >
                            <Download className="w-4 h-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setFileToDelete({ directory: selectedDirectory, filename: file.name })}
                          >
                            <Trash2 className="w-4 h-4 text-destructive" />
                          </Button>
                        </div>
                      </div>
                    ))}
                  </div>
                </>
              )}
            </div>
          </>
        )}
      </CardContent>

      {/* Delete File Dialog */}
      <Dialog open={fileToDelete !== null} onOpenChange={(open) => !open && setFileToDelete(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete File</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete "{fileToDelete?.filename}"? This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          {deleteMutation.isError && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />
              <AlertDescription>
                {deleteMutation.error instanceof Error ? deleteMutation.error.message : 'Failed to delete file'}
              </AlertDescription>
            </Alert>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setFileToDelete(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                if (fileToDelete) {
                  deleteMutation.mutate(fileToDelete);
                }
              }}
              disabled={deleteMutation.isPending}
            >
              {deleteMutation.isPending ? (
                <>
                  <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                  Deleting...
                </>
              ) : (
                <>
                  <Trash2 className="w-4 h-4 mr-2" />
                  Delete
                </>
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Delete All Logs Dialog */}
      <Dialog open={showDeleteAllDialog} onOpenChange={setShowDeleteAllDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete All Logs</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete ALL log files? This will permanently delete {files.length} file
              {files.length !== 1 ? 's' : ''} ({formatFileSize(totalSize)}). This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          {deleteAllLogsMutation.isError && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />
              <AlertDescription>
                {deleteAllLogsMutation.error instanceof Error
                  ? deleteAllLogsMutation.error.message
                  : 'Failed to delete logs'}
              </AlertDescription>
            </Alert>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteAllDialog(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => deleteAllLogsMutation.mutate()}
              disabled={deleteAllLogsMutation.isPending}
            >
              {deleteAllLogsMutation.isPending ? (
                <>
                  <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                  Deleting...
                </>
              ) : (
                <>
                  <Trash2 className="w-4 h-4 mr-2" />
                  Delete All
                </>
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Upload File Dialog */}
      <Dialog open={showUploadDialog} onOpenChange={setShowUploadDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Upload File</DialogTitle>
            <DialogDescription>
              Upload a file to the {uploadDirectory} directory on your Pico
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label htmlFor="upload-file">Select File</Label>
              <Input
                id="upload-file"
                type="file"
                accept={uploadDirectory === 'profiles' ? '.json' : '.csv,.log,.txt'}
                onChange={(e) => setUploadFile(e.target.files?.[0] || null)}
                className="mt-1"
              />
              {uploadFile && (
                <p className="text-sm text-muted-foreground mt-2">
                  Selected: {uploadFile.name} ({formatFileSize(uploadFile.size)})
                </p>
              )}
            </div>
            {uploadMutation.isError && (
              <Alert variant="destructive">
                <AlertCircle className="h-4 w-4" />
                <AlertDescription>
                  {uploadMutation.error instanceof Error
                    ? uploadMutation.error.message
                    : 'Failed to upload file'}
                </AlertDescription>
              </Alert>
            )}
            {uploadMutation.isSuccess && (
              <Alert>
                <CheckCircle className="h-4 w-4" />
                <AlertDescription>File uploaded successfully!</AlertDescription>
              </Alert>
            )}
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setShowUploadDialog(false);
                setUploadFile(null);
              }}
            >
              Cancel
            </Button>
            <Button
              onClick={handleUploadFile}
              disabled={!uploadFile || uploadMutation.isPending}
            >
              {uploadMutation.isPending ? (
                <>
                  <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                  Uploading...
                </>
              ) : (
                <>
                  <Upload className="w-4 h-4 mr-2" />
                  Upload
                </>
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
