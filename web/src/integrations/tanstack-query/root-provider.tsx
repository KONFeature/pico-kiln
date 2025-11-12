import { createAsyncStoragePersister } from "@tanstack/query-async-storage-persister";
import { QueryClient } from "@tanstack/react-query";
import { PersistQueryClientProvider } from "@tanstack/react-query-persist-client";

// Create async storage persister using localStorage
const asyncStoragePersister = createAsyncStoragePersister({
	storage: {
		getItem: async (key) => {
			const value = localStorage.getItem(key);
			return value ? JSON.parse(value) : null;
		},
		setItem: async (key, value) => {
			localStorage.setItem(key, JSON.stringify(value));
		},
		removeItem: async (key) => {
			localStorage.removeItem(key);
		},
	},
});

export function getContext() {
	const queryClient = new QueryClient({
		defaultOptions: {
			queries: {
				// Increase stale time for file-related queries since they don't change often
				staleTime: 1000 * 60 * 5, // 5 minutes
				gcTime: 1000 * 60 * 60 * 24, // 24 hours (previously cacheTime)
			},
		},
	});
	return {
		queryClient,
	};
}

export function Provider({
	children,
	queryClient,
}: {
	children: React.ReactNode;
	queryClient: QueryClient;
}) {
	return (
		<PersistQueryClientProvider
			client={queryClient}
			persistOptions={{
				persister: asyncStoragePersister,
				maxAge: 1000 * 60 * 60 * 24 * 7, // Persist for 7 days
				// Only persist file-related queries (profiles and logs listings/content)
				dehydrateOptions: {
					shouldDehydrateQuery: (query) => {
						const queryKey = query.queryKey;
						// Persist file-related queries
						if (queryKey[0] === "files" || queryKey[0] === "file-content") {
							return true;
						}
						// Don't persist status or other real-time queries
						return false;
					},
				},
			}}
		>
			{children}
		</PersistQueryClientProvider>
	);
}
