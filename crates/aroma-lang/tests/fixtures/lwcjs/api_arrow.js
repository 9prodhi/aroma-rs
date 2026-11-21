import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
@api clear = () => { this.value = null; };
}
