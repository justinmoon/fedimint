import { JsonRpcError, JsonRpcWebsocket } from 'jsonrpc-client-websocket';

export interface ApiInterface {
	loggedIn: () => boolean;
	setPassword: (password: string) => Promise<void>;
	setPasswordLocal: (password: string) => void;
	getDefaults: () => Promise<any>;
	setConnections: (ourName: string, leaderUrl?: string) => Promise<void>;
	setDefaults: (defaults: any) => Promise<void>;
	runDkg: () => Promise<void>;
	verify: () => Promise<void>;
	status: () => Promise<string>;
	startConsensus: () => Promise<void>;
}

async function rpc<T>(method: string, params: any, auth:string|null=null): Promise<T> {
	// FIXME: probably shouldn't have a default here ...
	const websocketUrl = process.env.REACT_APP_FM_CONFIG_API || 'ws://127.0.0.1:18174';
	const requestTimeoutMs = 20000;
	const websocket = new JsonRpcWebsocket(websocketUrl, requestTimeoutMs, (error: JsonRpcError) => {
		/* handle error */
		console.error('failed to create websocket', error);
	});
	await websocket.open();
	const response = await websocket.call(method, [{
		auth,
		params
	}]);
	websocket.close();
	return response.result as T;
}

export class Api implements ApiInterface {
	password: string | null;

	constructor() {
		this.password = null;
	}

	// FIXME: remove
	loggedIn = (): boolean => {
		return this.password !== null;
	};

	setPassword = async (password: string): Promise<void> => {
		await rpc('set_password', password);
		this.password = password;
	};

	setPasswordLocal = async (password: string) => {
		this.password = password;
	};

	getDefaults = async (): Promise<any> => {
		const defaults = await rpc('get_default_config_gen_params', null, this.password);
		console.log('get_default_config_gen_params result', defaults);
		return defaults;
	};

	setDefaults = async (defaults: any): Promise<void> => {
		const connections = {
			our_name: 'leader',
			leader_api_url: null,
		};
		const connectionsResult = await rpc('set_config_gen_connections', connections, this.password);
		console.log('set_config_gen_connections result', connectionsResult);
		const setResult = await rpc('set_config_gen_params', defaults, this.password);
		console.log('set_config_gen_params result', setResult);
	};

	setConnections = async (ourName: string, leaderUrl?: string): Promise<void> => {
		const connections = {
			our_name: ourName, // FIXME: make the call twice, second time with our_name
			leader_api_url: leaderUrl,
		};
		const defaults = await rpc('set_config_gen_connections', connections, this.password);
		console.log('set_config_gen_connections result', defaults);
	};

	runDkg = async (): Promise<void> => {
		const result = await rpc('run_dkg', null, this.password);
		console.log('runDkg result', result);
	};

	verify = async (): Promise<void> => {
		const hash = await rpc('get_verify_config_hash', null, this.password);
		console.log('get_verify_config_hash result', hash);
		const result = await rpc('verify_configs', [hash], this.password);
		console.log('verify_config result', result);
	};

	status = async (): Promise<string> => {
		const result = await rpc('status', null, this.password);
		console.log('status result', result);
		return result as string;
	};

	startConsensus = async (): Promise<void> => {
		const result = await rpc('start_consensus', this.password, this.password);
		console.log('start_consensus result', result);
	};
}
