import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
run(handler) { return handler?.(this.value); }
}
