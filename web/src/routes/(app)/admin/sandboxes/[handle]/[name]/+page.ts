import type { PageLoad } from './$types';

export const load: PageLoad = ({ params }) => ({ handle: params.handle, name: params.name });
