import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
first(items) { const [head, ...tail] = items; return [...tail, head]; }
}
