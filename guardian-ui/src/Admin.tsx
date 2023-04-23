import { Button, FormControl, FormLabel, Input, NumberDecrementStepper, NumberIncrementStepper, NumberInput, NumberInputField, NumberInputStepper, Radio, RadioGroup, Stack } from '@chakra-ui/react';
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

const LoggedIn = () => {
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
		} catch(e) {
			console.error('failed to set defaults', e);
		}
	}
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

	console.log(defaults);
	return (
		<>
			<Button onClick={onSetDefaults}>
				Set Defaults
			</Button>
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
			<div>
				{JSON.stringify(defaults)}
			</div>
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
			</>)}
		</>
	);
};

export enum Route {
	Login,
	LeadOrFollow,
	LeaderSetConsensusParameters,
	LeaderViewFollowers,
	FollowerSetLeader,
	FollowerAcceptConsensusParameters,
	Dkg,
	Hash,
	Consensus
}

export const Admin = () => {
	const [route, setRoute] = useState<Route>(Route.Login);
	function renderNavbar() {
		return (
			<nav>
				<Button onClick={() => setRoute(Route.Login)}>Login</Button>

				<Button onClick={() => setRoute(Route.LeadOrFollow)}>Lead Or Follow</Button>
				<Button onClick={() => setRoute(Route.LeaderSetConsensusParameters)}>Set Consensus Parameters</Button>
				<Button onClick={() => setRoute(Route.LeaderViewFollowers)}>View Followers</Button>

				<Button onClick={() => setRoute(Route.FollowerSetLeader)}>Set Leader</Button>
				<Button onClick={() => setRoute(Route.FollowerAcceptConsensusParameters)}>Accept Consensus Parameters</Button>

				<Button onClick={() => setRoute(Route.Dkg)}>DKG</Button>
				<Button onClick={() => setRoute(Route.Hash)}>Hash</Button>
				<Button onClick={() => setRoute(Route.Consensus)}>Consensus</Button>
			</nav>
		);
	}

	function renderRoute() {
		switch (route) {
		case Route.Login: {
			return <Login route={route} setRoute={setRoute} />;
		}
		default: {
			return <LoggedIn />;
		}
		}
	}


	return (
		<>
			{renderNavbar()}
			{renderRoute()}
		</>
	);
};