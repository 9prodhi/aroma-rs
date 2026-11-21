import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
valid(value) { return /^[a-z{}]+$/i.test(value); }
}
