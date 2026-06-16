import { createRouter } from "@tanstack/react-router";
import * as TanstackQuery from "./integrations/tanstack-query/root-provider";
import { ConfigDraftProvider } from "./lib/config/draft-context";
import { PicoProvider } from "./lib/pico/context";
import { ProfileCacheProvider } from "./lib/pico/profile-cache";
import { ThemeProvider } from "./lib/theme/theme-provider";

// Import the generated route tree
import { routeTree } from "./routeTree.gen";

// Create a new router instance
export const getRouter = () => {
	const rqContext = TanstackQuery.getContext();

	const router = createRouter({
		routeTree,
		context: { ...rqContext },
		defaultPreload: "intent",
		Wrap: (props: { children: React.ReactNode }) => {
			return (
				<ThemeProvider>
					<TanstackQuery.Provider {...rqContext}>
						<PicoProvider>
							<ConfigDraftProvider>
								<ProfileCacheProvider>{props.children}</ProfileCacheProvider>
							</ConfigDraftProvider>
						</PicoProvider>
					</TanstackQuery.Provider>
				</ThemeProvider>
			);
		},
	});

	return router;
};
