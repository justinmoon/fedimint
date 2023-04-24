import { Text, Button, FormControl, FormLabel, Input, NumberDecrementStepper, NumberIncrementStepper, NumberInput, NumberInputField, NumberInputStepper, Radio, RadioGroup, Stack } from '@chakra-ui/react';
import { useEffect, useState, useContext } from 'react';
import { ApiContext } from './components/ApiProvider';

interface RouteProps {
	route: Route,
	setRoute: React.Dispatch<React.SetStateAction<Route>>
}

const Login = (props: RouteProps) => {
	const { api } = useContext(ApiContext);
	const [password, setPassword] = useState('');

	async function onSignup() {
		try {
			await api.setPassword(password);
			console.log('password set');
			props.setRoute(Route.LeadOrFollow);
		} catch(e) {
			console.error('failed to set password', e);
		}
	}

	async function onLogin() {
		try {
			api.setPasswordLocal(password);
			console.log('password set');
			props.setRoute(Route.LeadOrFollow);
		} catch(e) {
			console.error('failed to set password', e);
		}
	}

	return (
		<>
			<Input placeholder='password' onChange={e => setPassword(e.target.value)}/>
			<Button onClick={onLogin}>
				Login
			</Button>
			<Button onClick={onSignup}>
				Signup
			</Button>
		</>

	);
};

const LeadOrFollow = (props: RouteProps) => {
	const { api } = useContext(ApiContext);

	async function onLead() {
		try {
			api.setConnections('leader');
			console.log('password set');
			props.setRoute(Route.LeaderAwaitPeers);
		} catch(e) {
			console.error('failed to set connections for the leader', e);
		}
	}

	return (
		<>
			<Button onClick={() => onLead()}>
				Lead
			</Button>
			<Button onClick={() => props.setRoute(Route.FollowerSetLeader)}>
				Follow
			</Button>
		</>

	);
};

const LeaderAwaitPeers = (props: RouteProps) => {
	const { api } = useContext(ApiContext);
	const [numPeers, setNumPeers] = useState(2);

	async function onAwaitPeers() {
		// TODO: retry if this times out or something
		await api.awaitPeers(numPeers);
		console.log('peers arrived');
		props.setRoute(Route.LeaderSetConsensusParameters);
	}

	// TODO: do some setInterval calling get_config_gen_peers to show who has joined so far
	return (
		<>
			<FormControl>
				<FormLabel>How many many guardians including yourself?</FormLabel>
				<NumberInput defaultValue={numPeers} min={1} max={20} onChange={n => setNumPeers(parseInt(n))}>
					<NumberInputField />
					<NumberInputStepper>
						<NumberIncrementStepper />
						<NumberDecrementStepper />
					</NumberInputStepper>
				</NumberInput>
			</FormControl>
			<Button onClick={onAwaitPeers}>
				Continue
			</Button>
		</>
	);
};


const LeaderSetConsensusParameters = (props: RouteProps) => {
	const { api } = useContext(ApiContext);
	const [defaults, setDefaults] = useState<any>(null);

	useEffect(() => {
		async function getDefaults() {
			const defaults = await api.getDefaults();
			setDefaults(defaults);
		}
		getDefaults();
	}, []);

	async function onSetDefaults() {
		try {
			await api.setDefaults(defaults);
			console.log('defaults set');
			props.setRoute(Route.Dkg);
		} catch(e) {
			console.error('failed to set defaults', e);
		}
	}

	function setFederationName(name: string) {
		const d = { ...defaults };
		d.meta.federation_name = name;
		setDefaults(d);
	}

	function setFinalityDelay(finalityDelay: string) {
		const d = { ...defaults };
		d.modules.wallet.finality_delay = parseInt(finalityDelay) - 1; // FIXME: is that right?
		setDefaults(d);
	}

	function setNetwork(network: string) {
		console.log(network);
		const d = { ...defaults };
		d.modules.wallet.network = network;
		setDefaults(d);
	}

	return (
		<>
			{defaults && (<> 
				<FormControl>
					<FormLabel>Federation Name</FormLabel>
					<Input value={defaults.meta.federation_name} onChange={e => setFederationName(e.target.value)}/>
				</FormControl>
				<FormControl>
					<FormLabel>Block Confirmations Required</FormLabel>
					<NumberInput defaultValue={defaults.modules.wallet.finality_delay} min={3} max={1000} onChange={n => setFinalityDelay(n)}>
						<NumberInputField />
						<NumberInputStepper>
							<NumberIncrementStepper />
							<NumberDecrementStepper />
						</NumberInputStepper>
					</NumberInput>
				</FormControl>
				<FormControl>
					<FormLabel>Network</FormLabel>
					<RadioGroup onChange={setNetwork} value={defaults.modules.wallet.network}>
						<Stack direction='row'>
							<Radio value='bitcoin'>Mainnet</Radio>
							<Radio value='testnet'>Testnet</Radio>
							<Radio value='signet'>Signet</Radio>
							<Radio value='regtest'>Regtest</Radio>
						</Stack>
					</RadioGroup>
				</FormControl>
				<Button onClick={onSetDefaults}>
					Continue
				</Button>
			</>)}
		</>
	);
};

const FollowerAcceptConsensusParameters = (props: RouteProps) => {
	const { api } = useContext(ApiContext);
	const [defaults, setDefaults] = useState<any>(null);

	useEffect(() => {
		async function getDefaults() {
			const defaults = await api.getDefaults();
			setDefaults(defaults);
		}
		getDefaults();
	}, []);


	async function onAccept() {
		try {
			await api.setDefaults(defaults);
			console.log('defaults set');
			props.setRoute(Route.Dkg);
		} catch(e) {
			console.error('failed to set defaults', e);
		}
	}

	return (
		<>
			{defaults && (<> 
				<FormControl>
					<FormLabel>Federation Name</FormLabel>
					<Input disabled={true} value={defaults.meta.federation_name} />
				</FormControl>
				<FormControl>
					<FormLabel>Block Confirmations Required</FormLabel>
					<Text>
						{defaults.modules.wallet.finality_delay}
					</Text> 
				</FormControl>
				<FormControl>
					<FormLabel>Network</FormLabel>
					<Text>
						{defaults.modules.wallet.network}
					</Text>
				</FormControl>
				<Button onClick={onAccept}>
					Accept
				</Button>
			</>)}
		</>
	);
};

const FollowerSetLeader = (props: RouteProps) => {
	const { api } = useContext(ApiContext);
	const [leaderUrl, setLeaderUrl] = useState('ws://127.0.0.1:18174');
	const [ourName, setOurName] = useState('follower');

	async function onSelectLeader() {
		try {
			await api.setConnections(ourName, leaderUrl);
			console.log('set connections');
			props.setRoute(Route.FollowerAcceptConsensusParameters);
		} catch(e) {
			console.error('failed to set connections', e);
		}
	}

	return (
		<>
			<Input placeholder='our name' value={ourName} onChange={e => setOurName(e.target.value)}/>
			<Input placeholder='leader url' value={leaderUrl} onChange={e => setLeaderUrl(e.target.value)}/>
			<Button onClick={onSelectLeader}>
				Set Leader URL
			</Button>
		</>
	);
};


const DkgAndConsensus = (props: RouteProps) => {
	const { api } = useContext(ApiContext);

	async function onDkg() {
		try {
			await api.runDkg();
			console.log('ran dkg');
		} catch(e) {
			console.error('failed to run dkg', e);
		}
	}

	async function onVerify() {
		try {
			const status = await api.verify();
			console.log('verify', status);
		} catch(e) {
			console.error('failed to verify', e);
		}
	}

	async function onStatus() {
		try {
			const status = await api.status();
			console.log('status', status);
		} catch(e) {
			console.error('failed to get status', e);
		}
	}

	async function onStartConsensus() {
		try {
			const status = await api.startConsensus();
			console.log('startConsensus', status);
		} catch(e) {
			console.error('failed to startConsensus', e);
		}
	}

	return (
		<>
			<Button onClick={onDkg}>
				Run DKG
			</Button>
			<Button onClick={onVerify}>
				Verify
			</Button>
			<Button onClick={onStatus}>
				Status
			</Button>
			<Button onClick={onStartConsensus}>
				Start Consensus
			</Button>
		</>
	);
};

const Catchall = (props: RouteProps) => {
	return (
		<>
			Route: {props.route.toString()}
		</>

	);
};

export enum Route {
	Login,
	LeadOrFollow,
	LeaderSetConsensusParameters,
	LeaderAwaitPeers,
	// LeaderViewFollowers,
	FollowerSetLeader,
	FollowerAcceptConsensusParameters,
	Dkg,
	Hash,
	Consensus
}

export const Admin = () => {
	const [route, setRoute] = useState<Route>(Route.Login);
	const { api } = useContext(ApiContext);

	function renderNavbar() {
		return (
			<nav>
				<Button onClick={() => setRoute(Route.Login)}>Login</Button>

				<Button onClick={() => setRoute(Route.LeadOrFollow)}>Lead Or Follow</Button>
				<Button onClick={() => setRoute(Route.LeaderSetConsensusParameters)}>Set Consensus Parameters</Button>

				<Button onClick={() => setRoute(Route.FollowerSetLeader)}>Set Leader</Button>
				<Button onClick={() => setRoute(Route.FollowerAcceptConsensusParameters)}>Accept Consensus Parameters</Button>

				<Button onClick={() => setRoute(Route.Dkg)}>DKG</Button>
				<Button onClick={() => setRoute(Route.Hash)}>Hash</Button>
				<Button onClick={() => setRoute(Route.Consensus)}>Consensus</Button>
			</nav>
		);
	}

	async function onStatus() {
		try {
			const status = await api.status();
			console.log('status', status);
		} catch(e) {
			console.error('failed to get status', e);
		}
	}

	function renderRoute() {
		switch (route) {
		case Route.Login: {
			return <Login route={route} setRoute={setRoute} />;
		}
		case Route.LeadOrFollow: {
			return <LeadOrFollow route={route} setRoute={setRoute} />;
		}
		case Route.LeaderSetConsensusParameters: {
			return <LeaderSetConsensusParameters route={route} setRoute={setRoute} />;
		}
		case Route.LeaderAwaitPeers: {
			return <LeaderAwaitPeers route={route} setRoute={setRoute} />;
		}
		case Route.FollowerSetLeader: {
			return <FollowerSetLeader route={route} setRoute={setRoute} />;
		}
		case Route.FollowerAcceptConsensusParameters: {
			return <FollowerAcceptConsensusParameters route={route} setRoute={setRoute} />;
		}
		case Route.Dkg: {
			return <DkgAndConsensus route={route} setRoute={setRoute} />;
		}
		default: {
			return <Catchall route={route} setRoute={setRoute} />;
		}
		}
	}


	return (
		<>
			{renderNavbar()}
			<br/>
			{renderRoute()}
			<div>
				<Button onClick={onStatus}>
					Status
				</Button>
			</div>
		</>
	);
};